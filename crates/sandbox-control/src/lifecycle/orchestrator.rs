//! The `Lifecycle` orchestrator and its ports.
//!
//! Callers identify a Task and request a transition. The orchestrator resolves
//! intent, enforces the Closed guard, allocates (or, on recovery, reuses) an
//! operation generation, persists intent before the runtime effect, issues a
//! best-effort graceful `StopAgent` before the runtime effect on stop/remove,
//! invokes the runtime seam, and finalizes through one operation-aware
//! completion path: completion facts are persisted and the marker cleared in a
//! single transaction before the event is published; a pure no-op against an
//! already-converged target clears its marker synchronously.
//!
//! Multi-step transitions run **detached** from the caller future (spawned and
//! awaited through a join handle), so a caller disconnect cannot cancel a
//! half-applied transition.
//!
//! Serialization, durable generation allocation, and intent persistence are
//! expressed as ports so the ordering and decision logic here is exercised
//! against in-memory doubles; the Postgres implementations
//! (`pg_try_advisory_lock` on a dedicated pool, the single-statement intent
//! write, the commit-before-publication completion commit) satisfy the same ports.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::Instrument;

use moatless_backend_core::sandbox as core_sandbox;

use crate::error::SandboxError;
use moatless_tasks::task::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};
use moatless_tasks::task::sandbox_lifecycle::{SandboxLifecycle, SandboxOpError};

use super::error::LifecycleError;
use super::events::{self, EventContext, EventFact, FailureDetail, LifecycleEvent};
use super::outcome::LifecycleOutcome;
use super::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
use super::types::{
    ArchiveRetention, CorrelationMarker, IntentEffect, ObservedStatus, OperationGeneration,
    OperationKind, RuntimeEffect,
};

/// Persisted lifecycle intent for one Task.
#[derive(Debug, Clone)]
pub struct TaskIntent {
    pub status: TaskStatus,
    pub desired_state: SandboxDesiredState,
    pub stop_reason: Option<SandboxStopReason>,
    /// Retention persisted alongside a `Removed` intent so a crashed remove is
    /// retried with the original policy.
    pub retention: Option<ArchiveRetention>,
    pub marker: Option<CorrelationMarker>,
    pub desired_state_updated_at: DateTime<Utc>,
}

/// The pre-effect facts of a finished operation, handed to `finalize`. Groups
/// the operation's identity and intent result so completion derivation,
/// event-id scoping, and marker acknowledgement share one value.
struct OperationRun {
    kind: OperationKind,
    generation: OperationGeneration,
    intent_effect: IntentEffect,
    marker_was_incomplete: bool,
    stop_reason: Option<SandboxStopReason>,
}

/// The atomic intent write — desired state, reason, retention, and the
/// correlation marker, all in one statement/transaction so readers never
/// observe partial intent.
#[derive(Debug, Clone)]
pub struct IntentWrite {
    pub desired_state: SandboxDesiredState,
    pub stop_reason: Option<SandboxStopReason>,
    pub retention: Option<ArchiveRetention>,
    pub marker: CorrelationMarker,
}

/// A store-layer failure. Carries a human context and, when the backend
/// provides one, the underlying source (e.g. a `sqlx::Error`), so the
/// operation-specific [`LifecycleError`] the orchestrator raises does not
/// flatten it to an opaque string.
#[derive(Debug, thiserror::Error)]
#[error("{context}")]
pub struct StoreError {
    pub context: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl StoreError {
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            source: None,
        }
    }

    pub fn with_source(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// Durable intent persistence + generation allocation.
#[async_trait]
pub trait LifecycleDao: Send + Sync {
    async fn load(&self, task_id: &str) -> Result<Option<TaskIntent>, StoreError>;

    /// DB-derived reconciler candidates: participating Tasks (Open/Error) that
    /// desire Running, plus any Task carrying an incomplete operation marker.
    /// The reconciler unions these with pending-message Tasks, then observes the
    /// fleet to decide which have actually diverged.
    async fn reconcile_candidates(&self) -> Result<Vec<String>, StoreError>;

    /// Allocate the next durable operation generation for the Task.
    async fn allocate_generation(&self, task_id: &str) -> Result<OperationGeneration, StoreError>;

    /// Atomically persist intent + marker. Returns whether intent actually
    /// changed (desired state, stop reason, or retention).
    async fn persist_intent(
        &self,
        task_id: &str,
        write: &IntentWrite,
    ) -> Result<IntentEffect, StoreError>;

    /// Persist a **full** deterministic terminal fact (id, status, and any
    /// reason tag) and clear the marker in one transaction. Used for ordinary
    /// completion and for a stale edge's durable abandonment fact. The caller
    /// publishes only after this returns `Ok`.
    async fn commit_fact_and_clear_marker(
        &self,
        task_id: &str,
        event: &LifecycleEvent,
    ) -> Result<(), StoreError>;

    /// Clear the marker synchronously — used for an `Unchanged` + `NoOp`
    /// against an already-converged target, where no completion fact will ever
    /// exist for the generation.
    async fn clear_marker(
        &self,
        task_id: &str,
        generation: OperationGeneration,
    ) -> Result<(), StoreError>;
}

/// The narrow internal agent-stop port. Graceful stop is lifecycle's ordering
/// rule: `stop`/`remove` issue a best-effort `StopAgent` **before** the runtime
/// effect so the pod exits promptly. Failures never block the transition.
#[async_trait]
pub trait AgentStopPort: Send + Sync {
    /// `reason` is forwarded to the sandbox so the terminal `agent.stopped`
    /// event attributes the stop to its initiator (user, idle reaper, ...).
    async fn stop_agent(&self, task_id: &str, reason: SandboxStopReason);
}

/// Publishes derived lifecycle events through the event-publication path.
#[async_trait]
pub trait LifecycleEventSink: Send + Sync {
    async fn publish(&self, task_id: &str, event: &LifecycleEvent);
    async fn publish_diagnostics(
        &self,
        task_id: &str,
        generation: OperationGeneration,
        observed: ObservedStatus,
        diagnostics: &super::outcome::Diagnostics,
    );
}

/// A held per-Task lock. Dropping it releases the lock.
pub trait LockGuard: Send {}

/// Per-Task serialization across backend replicas.
///
/// Every operation takes the lock — there is no lock-free fast path. The
/// Postgres implementation is `pg_try_advisory_lock` on a dedicated lock-only
/// connection pool with bounded retry; on exhaustion the orchestrator surfaces
/// `OperationInProgress { kind }` carrying the holder's kind.
#[async_trait]
pub trait LifecycleLock: Send + Sync {
    /// Try to acquire the Task's lock for `kind`. On contention, returns the
    /// conflicting holder's `kind`.
    async fn try_acquire(
        &self,
        task_id: &str,
        kind: OperationKind,
    ) -> Result<Box<dyn LockGuard>, OperationKind>;
}

/// A wall-clock source, used for marker timestamps. Named `WallClock` because
/// the representation is `DateTime<Utc>` (wall time via `Utc::now()`), which
/// can move backward under NTP adjustment — it is not monotonic.
pub trait WallClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Production wall clock.
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// The deep lifecycle module.
#[derive(Clone)]
pub struct Lifecycle {
    store: Arc<dyn LifecycleDao>,
    runtime: Arc<dyn SandboxRuntime>,
    agent_stop: Arc<dyn AgentStopPort>,
    events: Arc<dyn LifecycleEventSink>,
    lock: Arc<dyn LifecycleLock>,
    clock: Arc<dyn WallClock>,
    plan: Option<super::plan::ProvisionDeps>,
    default_sandbox_image: Option<String>,
    /// Bounded lock-acquire attempts before surfacing `OperationInProgress`.
    lock_attempts: u32,
}

impl Lifecycle {
    pub fn new(
        store: Arc<dyn LifecycleDao>,
        runtime: Arc<dyn SandboxRuntime>,
        agent_stop: Arc<dyn AgentStopPort>,
        events: Arc<dyn LifecycleEventSink>,
        lock: Arc<dyn LifecycleLock>,
        clock: Arc<dyn WallClock>,
    ) -> Self {
        Self {
            store,
            runtime,
            agent_stop,
            events,
            lock,
            clock,
            plan: None,
            default_sandbox_image: None,
            lock_attempts: 1,
        }
    }

    pub fn with_lock_attempts(mut self, attempts: u32) -> Self {
        self.lock_attempts = attempts.max(1);
        self
    }

    pub fn with_plan_knowledge(
        mut self,
        plan: super::plan::ProvisionDeps,
        default_sandbox_image: String,
    ) -> Self {
        self.plan = Some(plan);
        self.default_sandbox_image = Some(default_sandbox_image);
        self
    }

    /// Image a fresh start/redeploy would use, resolved through lifecycle's
    /// private workspace plan dependencies.
    pub async fn expected_sandbox_image(&self, task_id: &str) -> Result<String, LifecycleError> {
        let plan = self.plan.as_ref().ok_or_else(|| LifecycleError::Runtime {
            task_id: task_id.to_string(),
            source: super::error::RuntimeError::Infrastructure(
                "sandbox plan knowledge unavailable".to_string(),
            ),
        })?;
        let default_image =
            self.default_sandbox_image
                .as_deref()
                .ok_or_else(|| LifecycleError::Runtime {
                    task_id: task_id.to_string(),
                    source: super::error::RuntimeError::Infrastructure(
                        "default sandbox image unavailable".to_string(),
                    ),
                })?;
        plan.expected_sandbox_image(task_id, default_image)
            .await
            .map_err(|source| LifecycleError::Runtime {
                task_id: task_id.to_string(),
                source,
            })
    }

    /// Classify one watcher observation against durable intent and the current
    /// correlation marker. Reads fail closed at the watcher boundary: callers
    /// log the error and publish the observation rather than hiding it.
    pub(crate) async fn classify_observation(
        &self,
        task_id: &str,
        observation: super::classify::Observation,
    ) -> Result<super::classify::ClassificationResult, LifecycleError> {
        let intent = self.require_intent(task_id).await?;
        let suppression_active = intent.marker.is_some_and(|marker| {
            super::marker::edge_suppression_active(
                &marker,
                self.clock.now(),
                super::marker::EDGE_SUPPRESSION_STALE_AFTER,
            )
        });
        Ok(super::classify::classify_with_correlation(
            observation,
            intent.desired_state,
            intent.marker,
            suppression_active,
        ))
    }

    // ---- Public operations (detached from the caller future) ----

    pub async fn start(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let me = self.clone();
        let id = task_id.to_string();
        let span = tracing::Span::current();
        Self::join(
            task_id.to_string(),
            tokio::spawn(async move { me.op_start(id).await }.instrument(span)),
        )
        .await
    }

    pub async fn stop(
        &self,
        task_id: &str,
        reason: SandboxStopReason,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let me = self.clone();
        let id = task_id.to_string();
        let span = tracing::Span::current();
        Self::join(
            task_id.to_string(),
            tokio::spawn(async move { me.op_stop(id, reason).await }.instrument(span)),
        )
        .await
    }

    pub async fn restart(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let me = self.clone();
        let id = task_id.to_string();
        let span = tracing::Span::current();
        Self::join(
            task_id.to_string(),
            tokio::spawn(
                async move { me.op_edge(id, OperationKind::Restart).await }.instrument(span),
            ),
        )
        .await
    }

    pub async fn redeploy(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let me = self.clone();
        let id = task_id.to_string();
        let span = tracing::Span::current();
        Self::join(
            task_id.to_string(),
            tokio::spawn(
                async move { me.op_edge(id, OperationKind::Redeploy).await }.instrument(span),
            ),
        )
        .await
    }

    pub async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let me = self.clone();
        let id = task_id.to_string();
        let span = tracing::Span::current();
        Self::join(
            task_id.to_string(),
            tokio::spawn(async move { me.op_remove(id, retention).await }.instrument(span)),
        )
        .await
    }

    /// The persisted desired state for the Task's Sandbox — the status
    /// projections' and reaper's intent input, read from the Sandbox row through
    /// the store. Absence of a row reads as `Stopped` (never requested =
    /// converged Stopped); a missing Task is `TaskNotFound`.
    pub async fn desired_state(
        &self,
        task_id: &str,
    ) -> Result<SandboxDesiredState, LifecycleError> {
        Ok(self.require_intent(task_id).await?.desired_state)
    }

    /// Await a detached transition. Dropping the caller future drops this join
    /// handle, which does **not** cancel the spawned task — the transition runs
    /// to completion regardless.
    async fn join(
        task_id: String,
        handle: tokio::task::JoinHandle<Result<LifecycleOutcome, LifecycleError>>,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        match handle.await {
            Ok(result) => result,
            Err(join_err) => Err(LifecycleError::Interrupted {
                task_id,
                reason: join_err.to_string(),
            }),
        }
    }

    // ---- Detached bodies (own the lock guard for the transition) ----

    #[tracing::instrument(skip_all, fields(task_id = %task_id, op = "start"))]
    async fn op_start(self, task_id: String) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(&task_id, OperationKind::Start).await?;
        let intent = self.require_intent(&task_id).await?;
        Self::guard_toward_running(&task_id, OperationKind::Start, &intent)?;
        self.run_toward_running(&task_id, OperationKind::Start, &intent)
            .await
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id, op = ?kind))]
    async fn op_edge(
        self,
        task_id: String,
        kind: OperationKind,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(&task_id, kind).await?;
        let intent = self.require_intent(&task_id).await?;
        Self::guard_toward_running(&task_id, kind, &intent)?;
        self.run_toward_running(&task_id, kind, &intent).await
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id, op = "stop"))]
    async fn op_stop(
        self,
        task_id: String,
        reason: SandboxStopReason,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(&task_id, OperationKind::Stop).await?;
        let intent = self.require_intent(&task_id).await?;
        // stop is permitted in every status (idempotent for Closed).
        self.run_stop(&task_id, &intent, reason).await
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id, op = "remove"))]
    async fn op_remove(
        self,
        task_id: String,
        retention: ArchiveRetention,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(&task_id, OperationKind::Remove).await?;
        let intent = self.require_intent(&task_id).await?;
        // remove is permitted in every status.
        self.run_remove(&task_id, &intent, retention).await
    }

    // ---- Shared helpers ----

    async fn acquire(
        &self,
        task_id: &str,
        kind: OperationKind,
    ) -> Result<Box<dyn LockGuard>, LifecycleError> {
        let mut last_holder = kind;
        for _ in 0..self.lock_attempts {
            match self.lock.try_acquire(task_id, kind).await {
                Ok(guard) => return Ok(guard),
                Err(holder) => last_holder = holder,
            }
        }
        Err(LifecycleError::OperationInProgress { kind: last_holder })
    }

    async fn require_intent(&self, task_id: &str) -> Result<TaskIntent, LifecycleError> {
        self.store
            .load(task_id)
            .await
            .map_err(|e| LifecycleError::IntentLoad {
                task_id: task_id.to_string(),
                reason: e.to_string(),
            })?
            .ok_or_else(|| LifecycleError::TaskNotFound(task_id.to_string()))
    }

    /// The Closed guard is per-operation: start/restart/redeploy are rejected
    /// for a Closed task; stop/remove stay permitted (close flows persist
    /// `Closed` before removing the sandbox).
    fn guard_toward_running(
        task_id: &str,
        kind: OperationKind,
        intent: &TaskIntent,
    ) -> Result<(), LifecycleError> {
        if intent.status == TaskStatus::Closed {
            return Err(LifecycleError::NotAllowed {
                kind,
                task_id: task_id.to_string(),
                status: intent.status,
            });
        }
        Ok(())
    }

    /// Resolve the generation for an operation. A **recovery** — a level op
    /// whose target already matches persisted intent and whose marker is for
    /// the same kind — reuses the persisted generation so attempt identity and
    /// deterministic event ids stay stable across retries; the boolean marks
    /// that an incomplete marker preceded this attempt. Edges always allocate a
    /// fresh generation (they deliberately supersede).
    async fn resolve_generation(
        &self,
        task_id: &str,
        kind: OperationKind,
        intent: &TaskIntent,
        target: SandboxDesiredState,
    ) -> Result<(OperationGeneration, bool), LifecycleError> {
        if !kind.is_edge() {
            if let Some(marker) = intent.marker {
                if marker.kind == kind && intent.desired_state == target {
                    return Ok((marker.generation, true));
                }
            }
        }
        Ok((self.allocate(task_id).await?, false))
    }

    async fn allocate(&self, task_id: &str) -> Result<OperationGeneration, LifecycleError> {
        self.store.allocate_generation(task_id).await.map_err(|e| {
            LifecycleError::GenerationUnavailable {
                task_id: task_id.to_string(),
                reason: e.to_string(),
            }
        })
    }

    async fn persist(
        &self,
        task_id: &str,
        write: &IntentWrite,
    ) -> Result<IntentEffect, LifecycleError> {
        self.store
            .persist_intent(task_id, write)
            .await
            .map_err(|e| LifecycleError::IntentWrite {
                task_id: task_id.to_string(),
                reason: e.to_string(),
            })
    }

    async fn run_toward_running(
        &self,
        task_id: &str,
        kind: OperationKind,
        intent: &TaskIntent,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let target = SandboxDesiredState::Running;
        let (generation, marker_was_incomplete) = self
            .resolve_generation(task_id, kind, intent, target)
            .await?;
        let marker = CorrelationMarker::new(kind, generation, self.clock.now());
        let write = IntentWrite {
            desired_state: target,
            stop_reason: None,
            retention: None,
            marker,
        };
        let intent_effect = match self.persist(task_id, &write).await {
            Ok(effect) => effect,
            Err(e) => return Err(self.publish_failure(task_id, kind, generation, e).await),
        };

        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = match kind {
            OperationKind::Start => self.runtime.ensure_running(task_id, ctx).await,
            OperationKind::Restart => self.runtime.restart(task_id, ctx).await,
            OperationKind::Redeploy => self.runtime.redeploy(task_id, ctx).await,
            OperationKind::Stop | OperationKind::Remove => {
                unreachable!("toward-running kinds only")
            }
        };

        self.finalize(
            task_id,
            OperationRun {
                kind,
                generation,
                intent_effect,
                marker_was_incomplete,
                stop_reason: None,
            },
            runtime_result,
        )
        .await
    }

    async fn run_stop(
        &self,
        task_id: &str,
        intent: &TaskIntent,
        reason: SandboxStopReason,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        // stop is idempotent for a task already headed for removal: it must not
        // resurrect a `Removed` intent to `Stopped` or discard its retention
        // (which would drop a pending archive-deletion obligation).
        if intent.desired_state == SandboxDesiredState::Removed {
            return Ok(LifecycleOutcome::new(
                IntentEffect::Unchanged,
                RuntimeEffect::NoOp,
                ObservedStatus::Stopped,
            ));
        }

        let target = SandboxDesiredState::Stopped;
        let (generation, marker_was_incomplete) = self
            .resolve_generation(task_id, OperationKind::Stop, intent, target)
            .await?;
        let marker = CorrelationMarker::new(OperationKind::Stop, generation, self.clock.now());
        let write = IntentWrite {
            desired_state: target,
            stop_reason: Some(reason),
            retention: None,
            marker,
        };
        let intent_effect = match self.persist(task_id, &write).await {
            Ok(effect) => effect,
            Err(e) => {
                return Err(self
                    .publish_failure(task_id, OperationKind::Stop, generation, e)
                    .await);
            }
        };

        // Graceful agent stop BEFORE the runtime effect (best-effort).
        self.agent_stop.stop_agent(task_id, reason).await;

        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = self.runtime.stop(task_id, ctx).await;

        self.finalize(
            task_id,
            OperationRun {
                kind: OperationKind::Stop,
                generation,
                intent_effect,
                marker_was_incomplete,
                stop_reason: Some(reason),
            },
            runtime_result,
        )
        .await
    }

    async fn run_remove(
        &self,
        task_id: &str,
        intent: &TaskIntent,
        requested_retention: ArchiveRetention,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let target = SandboxDesiredState::Removed;
        let (generation, marker_was_incomplete) = self
            .resolve_generation(task_id, OperationKind::Remove, intent, target)
            .await?;
        // Recovery reuses the persisted retention; a fresh caller op uses the
        // supplied policy.
        let retention = if marker_was_incomplete {
            intent.retention.unwrap_or(requested_retention)
        } else {
            requested_retention
        };
        let marker = CorrelationMarker::new(OperationKind::Remove, generation, self.clock.now());
        let write = IntentWrite {
            desired_state: target,
            stop_reason: Some(SandboxStopReason::Cleanup),
            retention: Some(retention),
            marker,
        };
        let intent_effect = match self.persist(task_id, &write).await {
            Ok(effect) => effect,
            Err(e) => {
                return Err(self
                    .publish_failure(task_id, OperationKind::Remove, generation, e)
                    .await);
            }
        };

        // Acceptance event: intent moved to Removed (published before the
        // effect, so a later runtime failure still records that intent moved).
        let acc_ctx = EventContext {
            op: OperationKind::Remove,
            generation,
            intent_changed: intent_effect.changed(),
            runtime_applied: false,
            observed: ObservedStatus::Removed,
            marker_was_incomplete,
            stop_reason: None,
        };
        for event in events::acceptance_events(task_id, &acc_ctx) {
            self.events.publish(task_id, &event).await;
        }

        // Graceful agent stop BEFORE the runtime effect (best-effort).
        self.agent_stop
            .stop_agent(task_id, SandboxStopReason::Cleanup)
            .await;

        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = self.runtime.remove(task_id, retention, ctx).await;

        self.finalize(
            task_id,
            OperationRun {
                kind: OperationKind::Remove,
                generation,
                intent_effect,
                marker_was_incomplete,
                stop_reason: None,
            },
            runtime_result,
        )
        .await
    }

    /// The one operation-aware completion path.
    ///
    /// On runtime failure: publish the deterministic `Error` fact and return
    /// the typed error (intent and marker survive). On success: derive the
    /// operation's events; a completion fact (`Stopped`/`Removed`) is persisted
    /// and the marker cleared in one transaction **before** the event is
    /// published, so a commit failure propagates and nothing is published. If
    /// no completion fact is owed and the operation is a pure no-op against its
    /// already-converged target, clear the marker synchronously; otherwise the
    /// marker is left for the reconciler to acknowledge.
    async fn finalize(
        &self,
        task_id: &str,
        run: OperationRun,
        runtime_result: Result<RuntimeOutcome, super::error::RuntimeError>,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let runtime_outcome = match runtime_result {
            Ok(outcome) => outcome,
            Err(source) => {
                // Republish the adapter's own diagnosis. Without this the
                // durable fact says only "Sandbox operation failed" and the
                // reason survives nowhere but the caller's return value —
                // which, for a reconciler-driven operation, is a log line.
                let failure = FailureDetail::new(run.kind, source.cause(), source.to_string());
                self.events
                    .publish(
                        task_id,
                        &events::error_event(task_id, run.generation, failure),
                    )
                    .await;
                return Err(LifecycleError::Runtime {
                    task_id: task_id.to_string(),
                    source,
                });
            }
        };

        let outcome = LifecycleOutcome {
            intent_effect: run.intent_effect,
            runtime_effect: runtime_outcome.effect,
            observed: runtime_outcome.observed,
            diagnostics: runtime_outcome.diagnostics,
        };

        let ctx = EventContext::from_outcome(
            run.kind,
            run.generation,
            &outcome,
            run.marker_was_incomplete,
            run.stop_reason,
        );
        let mut committed_completion = false;
        for event in events::outcome_events(task_id, &ctx) {
            if matches!(event.fact, EventFact::Stopped | EventFact::Removed) {
                // Persist the full completion fact + clear the marker atomically;
                // propagate failure and do NOT publish the completion (intent +
                // marker survive for the reconciler, so the deterministic id is
                // never published without being persisted). The *failure* is
                // published, otherwise a completion that never committed leaves
                // a Task diverged with an empty event stream.
                if let Err(e) = self
                    .store
                    .commit_fact_and_clear_marker(task_id, &event)
                    .await
                {
                    return Err(self
                        .publish_failure(
                            task_id,
                            run.kind,
                            run.generation,
                            LifecycleError::CompletionCommit {
                                task_id: task_id.to_string(),
                                reason: e.to_string(),
                            },
                        )
                        .await);
                }
                committed_completion = true;
            }
            self.events.publish(task_id, &event).await;
        }
        if !outcome.diagnostics.is_empty() {
            self.events
                .publish_diagnostics(
                    task_id,
                    run.generation,
                    outcome.observed,
                    &outcome.diagnostics,
                )
                .await;
        }

        // Synchronous clear only for a pure no-op against the already-converged
        // target — no completion fact will ever exist for this generation, so
        // waiting for one would strand the marker.
        if !committed_completion
            && outcome.is_pure_no_op()
            && Self::observed_is_converged_target(run.kind, outcome.observed)
        {
            self.clear_marker_best_effort(task_id, run.generation).await;
        }

        Ok(outcome)
    }

    /// The observed status that means "converged" for an operation's target.
    fn observed_is_converged_target(kind: OperationKind, observed: ObservedStatus) -> bool {
        match kind {
            OperationKind::Start | OperationKind::Restart | OperationKind::Redeploy => {
                observed == ObservedStatus::Ready
            }
            OperationKind::Stop => observed == ObservedStatus::Stopped,
            OperationKind::Remove => observed == ObservedStatus::Removed,
        }
    }

    /// Publish the deterministic `Error` fact for a failure that already owns a
    /// generation, then hand the error back for propagation.
    ///
    /// Failures *before* generation resolution (intent load, generation
    /// allocation, lock contention, the Closed guard) have no stable event
    /// identity, so they would either need a random id — which defeats the
    /// dedup that keeps a retry loop from spamming the feed — or a shared one
    /// that collides across operations. They stay caller- and log-only.
    async fn publish_failure(
        &self,
        task_id: &str,
        kind: OperationKind,
        generation: OperationGeneration,
        error: LifecycleError,
    ) -> LifecycleError {
        let failure = FailureDetail::new(kind, error.cause(), error.to_string());
        self.events
            .publish(task_id, &events::error_event(task_id, generation, failure))
            .await;
        error
    }

    /// Publish the terminal `RecoveryExhausted` fact for a generation the
    /// reconciler has stopped retrying. Called by the reconciler, which owns
    /// the attempt budget; the event sink stays private to lifecycle.
    pub(crate) async fn publish_recovery_exhausted(
        &self,
        task_id: &str,
        kind: OperationKind,
        generation: OperationGeneration,
        attempts: u32,
        last_error: &str,
    ) {
        let failure = FailureDetail::new(
            kind,
            "recovery_exhausted",
            format!("{attempts} attempts failed, last error: {last_error}"),
        );
        self.events
            .publish(
                task_id,
                &events::recovery_exhausted_event(task_id, generation, failure),
            )
            .await;
    }

    async fn clear_marker_best_effort(&self, task_id: &str, generation: OperationGeneration) {
        // A no-op-generation marker owes no completion fact; if the clear fails
        // the reconciler's marker acknowledgement is the backstop.
        if let Err(e) = self.store.clear_marker(task_id, generation).await {
            tracing::warn!(task_id = %task_id, error = %e, "failed to clear no-op marker");
        }
    }

    /// Durably abandon a stale caller-owned edge. The deterministic `Error`
    /// fact and marker clear commit atomically, then the fact is published.
    ///
    /// `kind` is the abandoned edge's own operation, so the published fact names
    /// that operation rather than using a generic runtime-failure label.
    pub(crate) async fn abandon_edge(
        &self,
        task_id: &str,
        kind: OperationKind,
        generation: OperationGeneration,
    ) -> Result<(), LifecycleError> {
        let failure = FailureDetail::new(
            kind,
            "edge_abandoned",
            format!(
                "no replacement observed within {}s; the operation was abandoned",
                super::marker::EDGE_ABANDON_AFTER.as_secs()
            ),
        );
        let event = events::error_event(task_id, generation, failure);
        self.store
            .commit_fact_and_clear_marker(task_id, &event)
            .await
            .map_err(|e| LifecycleError::CompletionCommit {
                task_id: task_id.to_string(),
                reason: e.to_string(),
            })?;
        self.events.publish(task_id, &event).await;
        Ok(())
    }
}

// The durable-lifecycle ports of the sandbox runtime contract. `Lifecycle` is
// the sole writer of Sandbox intent; these hand a transport the one capability
// it needs without taking the whole orchestrator. A `LifecycleError` reaches
// the contract through this module's `SandboxError`, so the three client-visible
// lifecycle outcomes keep their mapping.
#[async_trait]
impl core_sandbox::SandboxIntentReader for Lifecycle {
    async fn desired_state(
        &self,
        task_id: &str,
    ) -> Result<SandboxDesiredState, core_sandbox::SandboxError> {
        Self::desired_state(self, task_id)
            .await
            .map_err(|e| SandboxError::from(e).into_contract())
    }
}

#[async_trait]
impl core_sandbox::SandboxStarter for Lifecycle {
    async fn start(&self, task_id: &str) -> Result<(), core_sandbox::SandboxError> {
        Self::start(self, task_id)
            .await
            .map(|_| ())
            .map_err(|e| SandboxError::from(e).into_contract())
    }
}

#[async_trait]
impl core_sandbox::SandboxStopper for Lifecycle {
    async fn stop(
        &self,
        task_id: &str,
        reason: SandboxStopReason,
    ) -> Result<(), core_sandbox::SandboxError> {
        Self::stop(self, task_id, reason)
            .await
            .map(|_| ())
            .map_err(|e| SandboxError::from(e).into_contract())
    }
}

/// Classify a `LifecycleError` for the tasks-band lifecycle port. This is the
/// `LifecycleError -> TaskError` mapping the tasks band used to own, moved to the
/// implementation side so the tasks band names no lifecycle error: the three
/// client-visible outcomes (404 / 400 / 409) keep their shape and everything
/// else is an internal failure with its rendered chain intact. Unlike the T3
/// contract's `SandboxError`, `SandboxOpError` has an `InProgress` variant, so
/// lock contention stays a 409 rather than flattening to a 500.
fn op_error(e: LifecycleError) -> SandboxOpError {
    use LifecycleError as LE;
    match &e {
        LE::TaskNotFound(id) => SandboxOpError::NotFound(id.clone()),
        LE::NotAllowed { .. } => SandboxOpError::NotAllowed(e.to_string()),
        LE::OperationInProgress { .. } => SandboxOpError::InProgress(e.to_string()),
        _ => SandboxOpError::Internal(e.to_string()),
    }
}

/// The tasks-band lifecycle port. The closer, reaper, HTTP handlers, and service
/// drive a Task's sandbox through this without naming a `sandbox` type; the four
/// methods are the whole tasks-band cross-boundary surface.
#[async_trait]
impl SandboxLifecycle for Lifecycle {
    async fn start(&self, task_id: &str) -> Result<(), SandboxOpError> {
        Self::start(self, task_id)
            .await
            .map(|_| ())
            .map_err(op_error)
    }

    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxOpError> {
        Self::stop(self, task_id, reason)
            .await
            .map(|_| ())
            .map_err(op_error)
    }

    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
    ) -> Result<(), SandboxOpError> {
        Self::remove(self, task_id, retention)
            .await
            .map(|_| ())
            .map_err(op_error)
    }

    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxOpError> {
        Self::desired_state(self, task_id).await.map_err(op_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::lifecycle::error::RuntimeError;
    use chrono::TimeZone;

    // ---- In-memory / recording doubles ----

    fn fixed_now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("valid ts")
    }

    struct TestClock;
    impl WallClock for TestClock {
        fn now(&self) -> DateTime<Utc> {
            fixed_now()
        }
    }

    /// Shared ordered log so tests can assert call ordering across ports.
    type Log = Arc<Mutex<Vec<String>>>;

    struct InMemoryLifecycleDao {
        intent: Mutex<Option<TaskIntent>>,
        next_gen: Mutex<i64>,
        commits: Mutex<Vec<String>>,
        marker_cleared: Mutex<bool>,
        fail_persist: bool,
        fail_commit: bool,
    }

    impl InMemoryLifecycleDao {
        fn with_intent(intent: TaskIntent) -> Arc<Self> {
            Arc::new(Self {
                intent: Mutex::new(Some(intent)),
                next_gen: Mutex::new(1),
                commits: Mutex::new(vec![]),
                marker_cleared: Mutex::new(false),
                fail_persist: false,
                fail_commit: false,
            })
        }
        fn missing() -> Arc<Self> {
            Arc::new(Self {
                intent: Mutex::new(None),
                next_gen: Mutex::new(1),
                commits: Mutex::new(vec![]),
                marker_cleared: Mutex::new(false),
                fail_persist: false,
                fail_commit: false,
            })
        }
        fn current(&self) -> Option<TaskIntent> {
            self.intent.lock().expect("lock").clone()
        }
        fn commits(&self) -> Vec<String> {
            self.commits.lock().expect("lock").clone()
        }
        fn marker_cleared(&self) -> bool {
            *self.marker_cleared.lock().expect("lock")
        }
    }

    #[async_trait]
    impl LifecycleDao for InMemoryLifecycleDao {
        async fn load(&self, _task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
            Ok(self.intent.lock().expect("lock").clone())
        }
        async fn reconcile_candidates(&self) -> Result<Vec<String>, StoreError> {
            Ok(vec![])
        }
        async fn allocate_generation(
            &self,
            _task_id: &str,
        ) -> Result<OperationGeneration, StoreError> {
            let mut g = self.next_gen.lock().expect("lock");
            let out = OperationGeneration(*g);
            *g += 1;
            Ok(out)
        }
        async fn persist_intent(
            &self,
            _task_id: &str,
            write: &IntentWrite,
        ) -> Result<IntentEffect, StoreError> {
            if self.fail_persist {
                return Err(StoreError::new("persist failed"));
            }
            let mut slot = self.intent.lock().expect("lock");
            let effect = match slot.as_ref() {
                Some(cur)
                    if cur.desired_state == write.desired_state
                        && cur.stop_reason == write.stop_reason
                        && cur.retention == write.retention =>
                {
                    IntentEffect::Unchanged
                }
                _ => IntentEffect::Changed,
            };
            let status = slot.as_ref().map_or(TaskStatus::Open, |c| c.status);
            *slot = Some(TaskIntent {
                status,
                desired_state: write.desired_state,
                stop_reason: write.stop_reason,
                retention: write.retention,
                marker: Some(write.marker),
                desired_state_updated_at: fixed_now(),
            });
            Ok(effect)
        }
        async fn commit_fact_and_clear_marker(
            &self,
            _task_id: &str,
            event: &LifecycleEvent,
        ) -> Result<(), StoreError> {
            if self.fail_commit {
                return Err(StoreError::new("commit failed"));
            }
            self.commits
                .lock()
                .expect("lock")
                .push(event.event_id.clone());
            // Committing a completion clears the marker atomically.
            if let Some(cur) = self.intent.lock().expect("lock").as_mut() {
                if cur
                    .marker
                    .is_some_and(|marker| marker.generation == event.generation)
                {
                    cur.marker = None;
                }
            }
            Ok(())
        }
        async fn clear_marker(
            &self,
            _task_id: &str,
            generation: OperationGeneration,
        ) -> Result<(), StoreError> {
            *self.marker_cleared.lock().expect("lock") = true;
            if let Some(cur) = self.intent.lock().expect("lock").as_mut() {
                if cur
                    .marker
                    .is_some_and(|marker| marker.generation == generation)
                {
                    cur.marker = None;
                }
            }
            Ok(())
        }
    }

    struct RecordingAgentStop {
        log: Log,
    }
    #[async_trait]
    impl AgentStopPort for RecordingAgentStop {
        async fn stop_agent(&self, _task_id: &str, _reason: SandboxStopReason) {
            self.log.lock().expect("lock").push("stop_agent".into());
        }
    }

    struct RecordingEvents {
        published: Mutex<Vec<LifecycleEvent>>,
        diagnostics: Mutex<Vec<super::super::outcome::Diagnostics>>,
    }
    impl RecordingEvents {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                published: Mutex::new(vec![]),
                diagnostics: Mutex::new(vec![]),
            })
        }
        fn facts(&self) -> Vec<EventFact> {
            self.published
                .lock()
                .expect("lock")
                .iter()
                .map(|e| e.fact)
                .collect()
        }
        fn ids(&self) -> Vec<String> {
            self.published
                .lock()
                .expect("lock")
                .iter()
                .map(|e| e.event_id.clone())
                .collect()
        }
    }
    #[async_trait]
    impl LifecycleEventSink for RecordingEvents {
        async fn publish(&self, _task_id: &str, event: &LifecycleEvent) {
            self.published.lock().expect("lock").push(event.clone());
        }

        async fn publish_diagnostics(
            &self,
            _task_id: &str,
            _generation: OperationGeneration,
            _observed: ObservedStatus,
            diagnostics: &super::super::outcome::Diagnostics,
        ) {
            self.diagnostics
                .lock()
                .expect("lock")
                .push(diagnostics.clone());
        }
    }

    /// Configurable recording runtime adapter.
    struct RecordingRuntime {
        log: Log,
        ensure: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
        stop_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
        restart_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
        redeploy_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
        remove_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
        last_ctx: Mutex<Option<RuntimeOperationContext>>,
    }
    impl RecordingRuntime {
        fn new(log: Log) -> Self {
            Self {
                log,
                ensure: Mutex::new(None),
                stop_res: Mutex::new(None),
                restart_res: Mutex::new(None),
                redeploy_res: Mutex::new(None),
                remove_res: Mutex::new(None),
                last_ctx: Mutex::new(None),
            }
        }
        fn take(
            slot: &Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
        ) -> Result<RuntimeOutcome, RuntimeError> {
            slot.lock()
                .expect("lock")
                .take()
                .unwrap_or_else(|| Ok(RuntimeOutcome::no_op(ObservedStatus::Ready)))
        }
    }
    #[async_trait]
    impl SandboxRuntime for RecordingRuntime {
        async fn ensure_running(
            &self,
            _t: &str,
            ctx: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, RuntimeError> {
            self.log.lock().expect("lock").push("ensure_running".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.ensure)
        }
        async fn stop(
            &self,
            _t: &str,
            ctx: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, RuntimeError> {
            self.log.lock().expect("lock").push("runtime_stop".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.stop_res)
        }
        async fn restart(
            &self,
            _t: &str,
            ctx: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, RuntimeError> {
            self.log
                .lock()
                .expect("lock")
                .push("runtime_restart".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.restart_res)
        }
        async fn redeploy(
            &self,
            _t: &str,
            ctx: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, RuntimeError> {
            self.log
                .lock()
                .expect("lock")
                .push("runtime_redeploy".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.redeploy_res)
        }
        async fn remove(
            &self,
            _t: &str,
            _r: ArchiveRetention,
            ctx: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, RuntimeError> {
            self.log.lock().expect("lock").push("runtime_remove".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.remove_res)
        }
    }

    struct TestGuard;
    impl LockGuard for TestGuard {}

    /// A lock that denies `denies` times, then grants; holder kind is fixed.
    struct FlakyLock {
        denies: Mutex<u32>,
        holder: OperationKind,
    }
    impl FlakyLock {
        fn free() -> Arc<Self> {
            Arc::new(Self {
                denies: Mutex::new(0),
                holder: OperationKind::Stop,
            })
        }
        fn denying(times: u32, holder: OperationKind) -> Arc<Self> {
            Arc::new(Self {
                denies: Mutex::new(times),
                holder,
            })
        }
    }
    #[async_trait]
    impl LifecycleLock for FlakyLock {
        async fn try_acquire(
            &self,
            _t: &str,
            _kind: OperationKind,
        ) -> Result<Box<dyn LockGuard>, OperationKind> {
            let mut d = self.denies.lock().expect("lock");
            if *d > 0 {
                *d -= 1;
                Err(self.holder)
            } else {
                Ok(Box::new(TestGuard))
            }
        }
    }

    // ---- Builders ----

    fn intent(
        status: TaskStatus,
        desired: SandboxDesiredState,
        reason: Option<SandboxStopReason>,
        retention: Option<ArchiveRetention>,
        marker: Option<CorrelationMarker>,
    ) -> TaskIntent {
        TaskIntent {
            status,
            desired_state: desired,
            stop_reason: reason,
            retention,
            marker,
            desired_state_updated_at: fixed_now(),
        }
    }

    fn open_running() -> TaskIntent {
        intent(
            TaskStatus::Open,
            SandboxDesiredState::Running,
            None,
            None,
            None,
        )
    }
    fn open_stopped() -> TaskIntent {
        intent(
            TaskStatus::Open,
            SandboxDesiredState::Stopped,
            Some(SandboxStopReason::User),
            None,
            None,
        )
    }
    fn closed_removed() -> TaskIntent {
        intent(
            TaskStatus::Closed,
            SandboxDesiredState::Removed,
            Some(SandboxStopReason::Cleanup),
            Some(ArchiveRetention::RetainArchives),
            None,
        )
    }

    struct Harness {
        lifecycle: Lifecycle,
        store: Arc<InMemoryLifecycleDao>,
        runtime: Arc<RecordingRuntime>,
        events: Arc<RecordingEvents>,
        log: Log,
    }

    fn harness_with(store: Arc<InMemoryLifecycleDao>, lock: Arc<dyn LifecycleLock>) -> Harness {
        let log: Log = Arc::new(Mutex::new(vec![]));
        let runtime = Arc::new(RecordingRuntime::new(log.clone()));
        let events = RecordingEvents::new();
        let agent_stop = Arc::new(RecordingAgentStop { log: log.clone() });
        let lifecycle = Lifecycle::new(
            store.clone(),
            runtime.clone(),
            agent_stop,
            events.clone(),
            lock,
            Arc::new(TestClock),
        );
        Harness {
            lifecycle,
            store,
            runtime,
            events,
            log,
        }
    }

    fn harness(intent: TaskIntent) -> Harness {
        harness_with(InMemoryLifecycleDao::with_intent(intent), FlakyLock::free())
    }

    // ============================ Tests ============================

    // ---- Eligibility matrix ----

    #[tokio::test]
    async fn closed_task_rejects_start_restart_redeploy() {
        for op in ["start", "restart", "redeploy"] {
            let h = harness(closed_removed());
            let res = match op {
                "start" => h.lifecycle.start("t").await,
                "restart" => h.lifecycle.restart("t").await,
                _ => h.lifecycle.redeploy("t").await,
            };
            assert!(
                matches!(res, Err(LifecycleError::NotAllowed { .. })),
                "{op}"
            );
            assert!(h.log.lock().expect("lock").is_empty(), "{op}");
        }
    }

    #[tokio::test]
    async fn error_status_start_is_resumable() {
        // Error tasks are resumable: a start succeeds.
        let h = harness(intent(
            TaskStatus::Error,
            SandboxDesiredState::Stopped,
            Some(SandboxStopReason::Error),
            None,
            None,
        ));
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)));
        let out = h.lifecycle.start("t").await.expect("ok");
        assert_eq!(out.observed, ObservedStatus::Starting);
        assert_eq!(
            h.store.current().expect("intent").desired_state,
            SandboxDesiredState::Running
        );
    }

    #[tokio::test]
    async fn closed_removed_stop_preserves_removed_intent() {
        // stop is idempotent for Closed, but must not clobber Removed intent or
        // discard its retention (the archive-deletion obligation).
        let h = harness(closed_removed());
        let out = h
            .lifecycle
            .stop("t", SandboxStopReason::User)
            .await
            .expect("ok");
        assert!(out.is_pure_no_op());
        let cur = h.store.current().expect("intent");
        assert_eq!(cur.desired_state, SandboxDesiredState::Removed);
        assert_eq!(cur.retention, Some(ArchiveRetention::RetainArchives));
        // No runtime effect, no events.
        assert!(h.log.lock().expect("lock").is_empty());
        assert!(h.events.facts().is_empty());
    }

    #[tokio::test]
    async fn closed_task_permits_remove() {
        let h = harness(closed_removed());
        h.runtime
            .remove_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Removed)));
        assert!(h
            .lifecycle
            .remove("t", ArchiveRetention::DeleteArchives)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn missing_task_is_not_found() {
        let h = harness_with(InMemoryLifecycleDao::missing(), FlakyLock::free());
        assert!(matches!(
            h.lifecycle.start("t").await,
            Err(LifecycleError::TaskNotFound(_))
        ));
    }

    // ---- Intent-before-effect ordering ----

    #[tokio::test]
    async fn start_persists_running_intent_before_runtime_effect() {
        let h = harness(open_stopped());
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)));
        let out = h.lifecycle.start("t").await.expect("ok");
        let cur = h.store.current().expect("intent");
        assert_eq!(cur.desired_state, SandboxDesiredState::Running);
        assert_eq!(cur.stop_reason, None);
        assert_eq!(out.observed, ObservedStatus::Starting);
        assert!(out.intent_effect.changed());
        assert!(out.runtime_effect.applied());
    }

    #[tokio::test]
    async fn intent_write_failure_yields_no_runtime_effect() {
        let store = Arc::new(InMemoryLifecycleDao {
            intent: Mutex::new(Some(open_stopped())),
            next_gen: Mutex::new(1),
            commits: Mutex::new(vec![]),
            marker_cleared: Mutex::new(false),
            fail_persist: true,
            fail_commit: false,
        });
        let h = harness_with(store, FlakyLock::free());
        let res = h.lifecycle.start("t").await;
        assert!(matches!(res, Err(LifecycleError::IntentWrite { .. })));
        assert!(h.log.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn load_failure_is_intent_load_not_write() {
        // A read failure must classify as IntentLoad, not IntentWrite.
        struct FailingLoad;
        #[async_trait]
        impl LifecycleDao for FailingLoad {
            async fn load(&self, _t: &str) -> Result<Option<TaskIntent>, StoreError> {
                Err(StoreError::new("conn reset"))
            }
            async fn reconcile_candidates(&self) -> Result<Vec<String>, StoreError> {
                Ok(vec![])
            }
            async fn allocate_generation(
                &self,
                _t: &str,
            ) -> Result<OperationGeneration, StoreError> {
                Ok(OperationGeneration(1))
            }
            async fn persist_intent(
                &self,
                _t: &str,
                _w: &IntentWrite,
            ) -> Result<IntentEffect, StoreError> {
                Ok(IntentEffect::Changed)
            }
            async fn commit_fact_and_clear_marker(
                &self,
                _t: &str,
                _e: &LifecycleEvent,
            ) -> Result<(), StoreError> {
                Ok(())
            }
            async fn clear_marker(
                &self,
                _t: &str,
                _generation: OperationGeneration,
            ) -> Result<(), StoreError> {
                Ok(())
            }
        }
        let log: Log = Arc::new(Mutex::new(vec![]));
        let lifecycle = Lifecycle::new(
            Arc::new(FailingLoad),
            Arc::new(RecordingRuntime::new(log.clone())),
            Arc::new(RecordingAgentStop { log }),
            RecordingEvents::new(),
            FlakyLock::free(),
            Arc::new(TestClock),
        );
        assert!(matches!(
            lifecycle.start("t").await,
            Err(LifecycleError::IntentLoad { .. })
        ));
    }

    // ---- Graceful stop ordering ----

    #[tokio::test]
    async fn stop_issues_agent_stop_before_runtime_effect() {
        let h = harness(open_running());
        h.runtime
            .stop_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
        h.lifecycle
            .stop("t", SandboxStopReason::User)
            .await
            .expect("ok");
        let log = h.log.lock().expect("lock").clone();
        let stop_agent = log
            .iter()
            .position(|s| s == "stop_agent")
            .expect("stop_agent");
        let runtime_stop = log
            .iter()
            .position(|s| s == "runtime_stop")
            .expect("runtime_stop");
        assert!(
            stop_agent < runtime_stop,
            "StopAgent must precede runtime stop: {log:?}"
        );
    }

    #[tokio::test]
    async fn remove_issues_agent_stop_before_runtime_effect() {
        let h = harness(open_running());
        h.runtime
            .remove_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Removed)));
        h.lifecycle
            .remove("t", ArchiveRetention::DeleteArchives)
            .await
            .expect("ok");
        let log = h.log.lock().expect("lock").clone();
        let stop_agent = log
            .iter()
            .position(|s| s == "stop_agent")
            .expect("stop_agent");
        let runtime_remove = log
            .iter()
            .position(|s| s == "runtime_remove")
            .expect("runtime_remove");
        assert!(
            stop_agent < runtime_remove,
            "StopAgent must precede runtime remove: {log:?}"
        );
    }

    // ---- No-op suppression ----

    #[tokio::test]
    async fn reusing_ready_sandbox_is_no_op_and_publishes_nothing() {
        let h = harness(open_running());
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Ready)));
        let out = h.lifecycle.start("t").await.expect("ok");
        assert!(out.is_pure_no_op());
        assert!(h.events.facts().is_empty());
        assert!(h.store.marker_cleared());
    }

    #[tokio::test]
    async fn start_noop_still_starting_does_not_sync_clear_marker() {
        // A no-op that observes Starting (reused a still-starting pod) is not
        // converged, so the marker is left for the watcher's Ready convergence.
        let h = harness(open_running());
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Starting)));
        let out = h.lifecycle.start("t").await.expect("ok");
        assert!(out.is_pure_no_op());
        assert!(!h.store.marker_cleared());
        assert!(h.store.current().expect("intent").marker.is_some());
    }

    // ---- Start matrix (representative) ----

    #[tokio::test]
    async fn start_ready_synchronously_publishes_initializing_not_ready() {
        let h = harness(open_stopped());
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Ready)));
        let out = h.lifecycle.start("t").await.expect("ok");
        assert_eq!(out.observed, ObservedStatus::Ready);
        assert!(h.events.facts().contains(&EventFact::Initializing));
        assert!(h
            .events
            .published
            .lock()
            .expect("lock")
            .iter()
            .all(|e| e.status != moatless_events::SandboxStatus::Ready));
    }

    #[tokio::test]
    async fn runtime_success_diagnostics_are_republished_by_lifecycle() {
        use super::super::outcome::{DegradationWarning, Diagnostics, ServerFailure};

        let h = harness(open_stopped());
        let diagnostics = Diagnostics {
            degradations: vec![DegradationWarning {
                code: "storage_degraded".into(),
                message: "using ephemeral storage".into(),
            }],
            server_failures: vec![ServerFailure {
                server: "preview".into(),
                message: "port was unavailable".into(),
            }],
        };
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)
                .with_diagnostics(diagnostics.clone())));

        let outcome = h.lifecycle.start("t").await.expect("ok");

        assert_eq!(outcome.diagnostics, diagnostics);
        assert_eq!(
            *h.events.diagnostics.lock().expect("lock"),
            vec![diagnostics]
        );
    }

    #[tokio::test]
    async fn start_runtime_failure_publishes_one_error_and_keeps_intent() {
        let h = harness(open_stopped());
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Err(RuntimeError::Infrastructure(
                "previous pod still terminating".into(),
            )));
        let res = h.lifecycle.start("t").await;
        assert!(matches!(res, Err(LifecycleError::Runtime { .. })));
        assert_eq!(
            h.store.current().expect("intent").desired_state,
            SandboxDesiredState::Running
        );
        assert_eq!(h.events.facts(), vec![EventFact::Error]);

        // The adapter's diagnosis must reach the durable fact. Publishing a
        // bare Error here left the reason only in the caller's return value —
        // a log line, for a reconciler-driven start.
        let payload = h.events.published.lock().expect("lock")[0].payload();
        assert_eq!(
            payload.message,
            "Sandbox start failed: runtime infrastructure error: previous pod still terminating"
        );
        assert!(payload
            .error
            .as_deref()
            .is_some_and(|e| e.contains("previous pod still terminating")));
        assert_eq!(
            payload.lifecycle_event.as_deref(),
            Some("runtime_infrastructure")
        );
    }

    // ---- Correlation: generation reaches the adapter ----

    #[tokio::test]
    async fn generation_reaches_the_runtime_context() {
        let h = harness(open_stopped());
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)));
        h.lifecycle.start("t").await.expect("ok");
        let ctx = h.runtime.last_ctx.lock().expect("lock").expect("ctx");
        assert_eq!(ctx.operation_generation, OperationGeneration(1));
    }

    // ---- Race matrix ----

    #[tokio::test]
    async fn contended_lock_surfaces_operation_in_progress_with_holder_kind() {
        let store = InMemoryLifecycleDao::with_intent(open_running());
        let h = harness_with(store, FlakyLock::denying(1, OperationKind::Stop));
        let res = h.lifecycle.start("t").await;
        match res {
            Err(LifecycleError::OperationInProgress { kind }) => {
                assert_eq!(kind, OperationKind::Stop);
            }
            other => panic!("expected OperationInProgress, got {other:?}"),
        }
        assert!(h.log.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn bounded_lock_retry_eventually_succeeds() {
        // Denies twice then grants; three attempts succeed.
        let store = InMemoryLifecycleDao::with_intent(open_running());
        let log: Log = Arc::new(Mutex::new(vec![]));
        let runtime = Arc::new(RecordingRuntime::new(log.clone()));
        runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Ready)));
        let lifecycle = Lifecycle::new(
            store,
            runtime,
            Arc::new(RecordingAgentStop { log }),
            RecordingEvents::new(),
            FlakyLock::denying(2, OperationKind::Stop),
            Arc::new(TestClock),
        )
        .with_lock_attempts(3);
        assert!(lifecycle.start("t").await.is_ok());
    }

    #[tokio::test]
    async fn exhausted_lock_retry_surfaces_operation_in_progress() {
        let store = InMemoryLifecycleDao::with_intent(open_running());
        let h_lock = FlakyLock::denying(10, OperationKind::Remove);
        let h = harness_with(store, h_lock);
        let res = h.lifecycle.with_lock_attempts(3).start("t").await;
        assert!(matches!(
            res,
            Err(LifecycleError::OperationInProgress {
                kind: OperationKind::Remove
            })
        ));
    }

    // ---- Stop completion + durability ----

    #[tokio::test]
    async fn stop_against_running_publishes_stopped_and_commits_completion() {
        let h = harness(open_running());
        h.runtime
            .stop_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
        h.lifecycle
            .stop("t", SandboxStopReason::IdleReaper)
            .await
            .expect("ok");
        assert!(h.events.facts().contains(&EventFact::Stopped));
        assert_eq!(h.store.commits(), vec!["sandbox:t:1:stopped".to_string()]);
        assert!(h.store.current().expect("intent").marker.is_none());
    }

    #[tokio::test]
    async fn stop_against_already_stopped_is_changed_noop_still_publishes() {
        let h = harness(open_stopped());
        h.runtime
            .stop_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Stopped)));
        let out = h
            .lifecycle
            .stop("t", SandboxStopReason::IdleReaper)
            .await
            .expect("ok");
        assert!(out.intent_effect.changed());
        assert!(!out.runtime_effect.applied());
        assert!(h.events.facts().contains(&EventFact::Stopped));
    }

    #[tokio::test]
    async fn completion_commit_failure_propagates_and_does_not_publish() {
        // Durability: a failed completion transaction must not publish the event
        // nor report success; intent + marker survive for the reconciler.
        let store = Arc::new(InMemoryLifecycleDao {
            intent: Mutex::new(Some(open_running())),
            next_gen: Mutex::new(1),
            commits: Mutex::new(vec![]),
            marker_cleared: Mutex::new(false),
            fail_persist: false,
            fail_commit: true,
        });
        let h = harness_with(store, FlakyLock::free());
        h.runtime
            .stop_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
        let res = h.lifecycle.stop("t", SandboxStopReason::User).await;
        assert!(matches!(res, Err(LifecycleError::CompletionCommit { .. })));
        // Stopped completion was NOT published; marker retained.
        assert!(!h.events.facts().contains(&EventFact::Stopped));
        assert!(h.store.current().expect("intent").marker.is_some());
    }

    // ---- Remove: acceptance + completion, strict success, recovery ----

    #[tokio::test]
    async fn successful_remove_emits_removing_then_removed_and_commits() {
        let h = harness(open_running());
        h.runtime
            .remove_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Removed)));
        h.lifecycle
            .remove("t", ArchiveRetention::DeleteArchives)
            .await
            .expect("ok");
        assert_eq!(
            h.events.facts(),
            vec![EventFact::Removing, EventFact::Removed]
        );
        assert_eq!(
            h.store.current().expect("intent").retention,
            Some(ArchiveRetention::DeleteArchives)
        );
        assert_eq!(h.store.commits(), vec!["sandbox:t:1:removed".to_string()]);
    }

    #[tokio::test]
    async fn failed_remove_keeps_removed_intent_and_publishes_error() {
        let h = harness(open_running());
        h.runtime
            .remove_res
            .lock()
            .expect("lock")
            .replace(Err(RuntimeError::NotAbsent("pvc".into())));
        let res = h
            .lifecycle
            .remove("t", ArchiveRetention::DeleteArchives)
            .await;
        assert!(matches!(res, Err(LifecycleError::Runtime { .. })));
        let cur = h.store.current().expect("intent");
        assert_eq!(cur.desired_state, SandboxDesiredState::Removed);
        assert_eq!(cur.retention, Some(ArchiveRetention::DeleteArchives));
        let facts = h.events.facts();
        assert!(facts.contains(&EventFact::Removing));
        assert!(facts.contains(&EventFact::Error));
        assert!(!facts.contains(&EventFact::Removed));
        assert!(h.store.commits().is_empty());
    }

    #[tokio::test]
    async fn remove_recovery_reuses_generation_and_persisted_retention() {
        // A prior remove persisted Removed + RetainArchives with an incomplete
        // Remove marker at generation 5. The recovery retry reuses generation 5
        // (stable deterministic id) and the persisted retention — it does not
        // allocate a fresh generation nor take the caller's DeleteArchives.
        let prior_marker =
            CorrelationMarker::new(OperationKind::Remove, OperationGeneration(5), fixed_now());
        let store = Arc::new(InMemoryLifecycleDao {
            intent: Mutex::new(Some(intent(
                TaskStatus::Open,
                SandboxDesiredState::Removed,
                Some(SandboxStopReason::Cleanup),
                Some(ArchiveRetention::RetainArchives),
                Some(prior_marker),
            ))),
            next_gen: Mutex::new(100), // would be used only if it allocated afresh
            commits: Mutex::new(vec![]),
            marker_cleared: Mutex::new(false),
            fail_persist: false,
            fail_commit: false,
        });
        let h = harness_with(store, FlakyLock::free());
        h.runtime
            .remove_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Removed)));
        // Caller passes DeleteArchives, but recovery must keep RetainArchives.
        h.lifecycle
            .remove("t", ArchiveRetention::DeleteArchives)
            .await
            .expect("ok");
        // Deterministic id reuses generation 5, not 100.
        assert_eq!(h.store.commits(), vec!["sandbox:t:5:removed".to_string()]);
        assert_eq!(
            h.store.current().expect("intent").retention,
            Some(ArchiveRetention::RetainArchives)
        );
    }

    #[tokio::test]
    async fn idempotent_re_remove_noop_clears_marker_synchronously() {
        // finish path for remove must have a pure-no-op clear: an Unchanged +
        // NoOp remove against an already-removed target (fresh marker, not a
        // recovery) would otherwise strand its marker forever.
        let h = harness(intent(
            TaskStatus::Open,
            SandboxDesiredState::Removed,
            Some(SandboxStopReason::Cleanup),
            Some(ArchiveRetention::DeleteArchives),
            None, // no prior marker → not a recovery; a fresh no-op op
        ));
        h.runtime
            .remove_res
            .lock()
            .expect("lock")
            .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Removed)));
        let out = h
            .lifecycle
            .remove("t", ArchiveRetention::DeleteArchives)
            .await
            .expect("ok");
        assert!(out.is_pure_no_op());
        // No completion fact owed; marker cleared synchronously, none stranded.
        assert!(h.store.marker_cleared());
        assert!(h.store.current().expect("intent").marker.is_none());
        assert!(h.store.commits().is_empty());
    }

    // ---- Deterministic event ids ----

    #[tokio::test]
    async fn error_event_id_is_generation_scoped() {
        let h = harness(open_stopped());
        h.runtime
            .ensure
            .lock()
            .expect("lock")
            .replace(Err(RuntimeError::Infrastructure("x".into())));
        let _ = h.lifecycle.start("t").await;
        assert_eq!(h.events.ids(), vec!["sandbox:t:1:error".to_string()]);
    }
}
