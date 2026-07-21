//! The `Lifecycle` orchestrator and its ports.
//!
//! Callers identify a Task and request a transition. The orchestrator resolves
//! intent, enforces the Closed guard, allocates a generation, **persists intent
//! before the runtime effect**, issues a best-effort graceful `StopAgent`
//! before the runtime effect on stop/remove, invokes the runtime seam, derives
//! the typed outcome and its operation-level events, and returns.
//!
//! Serialization, durable generation allocation, and intent persistence are
//! expressed as **ports** so the ordering and decision logic here is tested
//! against in-memory doubles. Their Postgres implementations
//! (`pg_try_advisory_lock` on a dedicated pool; the single-statement intent
//! write; the transactional-outbox completion commit) are the step-2 wiring
//! this module is built to receive.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::task::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};

use super::error::LifecycleError;
use super::events::{self, EventContext, LifecycleEvent};
use super::outcome::LifecycleOutcome;
use super::runtime::{RuntimeOperationContext, SandboxRuntime};
use super::types::{
    ArchiveRetention, CorrelationMarker, IntentEffect, ObservedStatus, OperationGeneration,
    OperationKind,
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

/// Store-layer failure (transient by nature — intent is written before any
/// runtime effect).
#[derive(Debug, thiserror::Error)]
#[error("lifecycle store error: {0}")]
pub struct StoreError(pub String);

/// Durable intent persistence + generation allocation.
#[async_trait]
pub trait LifecycleStore: Send + Sync {
    async fn load(&self, task_id: &str) -> Result<Option<TaskIntent>, StoreError>;

    /// Allocate the next durable operation generation for the Task.
    async fn allocate_generation(&self, task_id: &str) -> Result<OperationGeneration, StoreError>;

    /// Atomically persist intent + marker. Returns whether intent actually
    /// changed (desired state, stop reason, or retention).
    async fn persist_intent(
        &self,
        task_id: &str,
        write: &IntentWrite,
    ) -> Result<IntentEffect, StoreError>;

    /// Atomically persist a completion fact and clear the marker in one
    /// transaction (the transactional-outbox commit).
    async fn commit_completion(&self, task_id: &str, event_id: &str) -> Result<(), StoreError>;

    /// Clear the marker synchronously — used for an `Unchanged` + `NoOp`
    /// against an already-converged target, where no completion fact will ever
    /// exist for the generation.
    async fn clear_marker(&self, task_id: &str) -> Result<(), StoreError>;
}

/// The narrow internal agent-stop port. Graceful stop is lifecycle's ordering
/// rule: `stop`/`remove` issue a best-effort `StopAgent` **before** the runtime
/// effect so the pod exits promptly. Failures never block the transition.
#[async_trait]
pub trait AgentStopPort: Send + Sync {
    async fn stop_agent(&self, task_id: &str);
}

/// Publishes derived lifecycle events through the outbox path.
#[async_trait]
pub trait LifecycleEventSink: Send + Sync {
    async fn publish(&self, task_id: &str, event: &LifecycleEvent);
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

/// A monotonic clock port so marker timestamps are deterministic in tests.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// The deep lifecycle module.
#[derive(Clone)]
pub struct Lifecycle {
    store: Arc<dyn LifecycleStore>,
    runtime: Arc<dyn SandboxRuntime>,
    agent_stop: Arc<dyn AgentStopPort>,
    events: Arc<dyn LifecycleEventSink>,
    lock: Arc<dyn LifecycleLock>,
    clock: Arc<dyn Clock>,
    /// Bounded lock-acquire attempts before surfacing `OperationInProgress`.
    lock_attempts: u32,
}

impl Lifecycle {
    pub fn new(
        store: Arc<dyn LifecycleStore>,
        runtime: Arc<dyn SandboxRuntime>,
        agent_stop: Arc<dyn AgentStopPort>,
        events: Arc<dyn LifecycleEventSink>,
        lock: Arc<dyn LifecycleLock>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store,
            runtime,
            agent_stop,
            events,
            lock,
            clock,
            lock_attempts: 1,
        }
    }

    pub fn with_lock_attempts(mut self, attempts: u32) -> Self {
        self.lock_attempts = attempts.max(1);
        self
    }

    // ---- Public operations ----

    pub async fn start(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(task_id, OperationKind::Start).await?;
        let intent = self.require_intent(task_id).await?;
        Self::guard_toward_running(task_id, OperationKind::Start, &intent)?;
        self.run_toward_running(task_id, OperationKind::Start, &intent).await
    }

    pub async fn stop(
        &self,
        task_id: &str,
        reason: SandboxStopReason,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(task_id, OperationKind::Stop).await?;
        let intent = self.require_intent(task_id).await?;
        // stop is permitted in every status (idempotent for Closed).
        self.run_stop(task_id, &intent, reason).await
    }

    pub async fn restart(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(task_id, OperationKind::Restart).await?;
        let intent = self.require_intent(task_id).await?;
        Self::guard_toward_running(task_id, OperationKind::Restart, &intent)?;
        self.run_toward_running(task_id, OperationKind::Restart, &intent).await
    }

    pub async fn redeploy(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(task_id, OperationKind::Redeploy).await?;
        let intent = self.require_intent(task_id).await?;
        Self::guard_toward_running(task_id, OperationKind::Redeploy, &intent)?;
        self.run_toward_running(task_id, OperationKind::Redeploy, &intent).await
    }

    pub async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let _guard = self.acquire(task_id, OperationKind::Remove).await?;
        let intent = self.require_intent(task_id).await?;
        // remove is permitted in every status.
        self.run_remove(task_id, &intent, retention).await
    }

    // ---- Internals ----

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
            .map_err(|e| LifecycleError::IntentWrite {
                task_id: task_id.to_string(),
                reason: e.0,
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
                status: intent.status.as_str().to_string(),
            });
        }
        Ok(())
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
                reason: e.0,
            })
    }

    async fn run_toward_running(
        &self,
        task_id: &str,
        kind: OperationKind,
        _intent: &TaskIntent,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        // 1. Intent before effect: Running, clear stop reason, no retention.
        let generation = self.allocate(task_id).await?;
        let marker = CorrelationMarker::new(kind, generation, self.clock.now());
        let write = IntentWrite {
            desired_state: SandboxDesiredState::Running,
            stop_reason: None,
            retention: None,
            marker,
        };
        let intent_effect = self.persist(task_id, &write).await?;

        // 2. Runtime effect from a fresh plan / restart / redeploy.
        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = match kind {
            OperationKind::Start => self.runtime.ensure_running(task_id, ctx).await,
            OperationKind::Restart => self.runtime.restart(task_id, ctx).await,
            OperationKind::Redeploy => self.runtime.redeploy(task_id, ctx).await,
            OperationKind::Stop | OperationKind::Remove => unreachable!("toward-running kinds only"),
        };

        self.finish(task_id, kind, generation, intent_effect, None, runtime_result)
            .await
    }

    async fn run_stop(
        &self,
        task_id: &str,
        _intent: &TaskIntent,
        reason: SandboxStopReason,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let generation = self.allocate(task_id).await?;
        let marker = CorrelationMarker::new(OperationKind::Stop, generation, self.clock.now());
        let write = IntentWrite {
            desired_state: SandboxDesiredState::Stopped,
            stop_reason: Some(reason),
            retention: None,
            marker,
        };
        let intent_effect = self.persist(task_id, &write).await?;

        // Graceful agent stop BEFORE the runtime effect (best-effort).
        self.agent_stop.stop_agent(task_id).await;

        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = self.runtime.stop(task_id, ctx).await;

        self.finish(
            task_id,
            OperationKind::Stop,
            generation,
            intent_effect,
            Some(reason),
            runtime_result,
        )
        .await
    }

    async fn run_remove(
        &self,
        task_id: &str,
        intent: &TaskIntent,
        retention: ArchiveRetention,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        // A recovery of an incomplete remove reuses the persisted retention.
        // A fresh caller op uses the supplied retention. We honor the supplied
        // one here (callers pass the intended policy); the reconciler's retry
        // path reuses `intent.retention`.
        let marker_was_incomplete = intent.desired_state == SandboxDesiredState::Removed
            && intent.marker.is_some_and(|m| m.kind == OperationKind::Remove);

        let generation = self.allocate(task_id).await?;
        let marker = CorrelationMarker::new(OperationKind::Remove, generation, self.clock.now());
        let write = IntentWrite {
            desired_state: SandboxDesiredState::Removed,
            stop_reason: Some(SandboxStopReason::Cleanup),
            retention: Some(retention),
            marker,
        };
        let intent_effect = self.persist(task_id, &write).await?;

        // Acceptance event: intent moved to Removed.
        let acc_ctx = EventContext {
            op: OperationKind::Remove,
            generation,
            intent_changed: intent_effect.changed(),
            runtime_applied: false,
            observed: ObservedStatus::Removed,
            marker_was_incomplete,
            stop_reason: None,
        };
        for e in events::acceptance_events(task_id, &acc_ctx) {
            self.events.publish(task_id, &e).await;
        }

        // Graceful agent stop BEFORE the runtime effect (best-effort).
        self.agent_stop.stop_agent(task_id).await;

        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = self.runtime.remove(task_id, retention, ctx).await;

        self.finish_remove(
            task_id,
            generation,
            intent_effect,
            marker_was_incomplete,
            runtime_result,
        )
        .await
    }

    async fn allocate(&self, task_id: &str) -> Result<OperationGeneration, LifecycleError> {
        self.store
            .allocate_generation(task_id)
            .await
            .map_err(|e| LifecycleError::IntentWrite {
                task_id: task_id.to_string(),
                reason: e.0,
            })
    }

    /// Shared finish for start/stop/restart/redeploy.
    async fn finish(
        &self,
        task_id: &str,
        kind: OperationKind,
        generation: OperationGeneration,
        intent_effect: IntentEffect,
        stop_reason: Option<SandboxStopReason>,
        runtime_result: Result<super::runtime::RuntimeOutcome, super::error::RuntimeError>,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let runtime_outcome = match runtime_result {
            Ok(o) => o,
            Err(source) => {
                // Failure fact for this generation; intent stays the target.
                self.events
                    .publish(task_id, &events::error_event(task_id, generation))
                    .await;
                return Err(LifecycleError::Runtime {
                    task_id: task_id.to_string(),
                    source,
                });
            }
        };

        let outcome = LifecycleOutcome {
            intent_effect,
            runtime_effect: runtime_outcome.effect,
            observed: runtime_outcome.observed,
            diagnostics: runtime_outcome.diagnostics,
        };

        // Pure no-op → clear the marker synchronously and publish nothing.
        if outcome.is_pure_no_op() {
            self.clear_marker_best_effort(task_id).await;
            return Ok(outcome);
        }

        let ctx = EventContext::from_outcome(kind, generation, &outcome, false, stop_reason);
        for e in events::outcome_events(task_id, &ctx) {
            self.publish_completion_or_infra(task_id, &e).await;
        }

        Ok(outcome)
    }

    /// Remove finish: strict success is the adapter's contract (it awaited
    /// observed absence). On success we commit the `Removed` completion and
    /// clear the marker in one transaction; on failure the marker and `Removed`
    /// intent survive for the reconciler.
    async fn finish_remove(
        &self,
        task_id: &str,
        generation: OperationGeneration,
        intent_effect: IntentEffect,
        marker_was_incomplete: bool,
        runtime_result: Result<super::runtime::RuntimeOutcome, super::error::RuntimeError>,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let runtime_outcome = match runtime_result {
            Ok(o) => o,
            Err(source) => {
                self.events
                    .publish(task_id, &events::error_event(task_id, generation))
                    .await;
                return Err(LifecycleError::Runtime {
                    task_id: task_id.to_string(),
                    source,
                });
            }
        };

        let outcome = LifecycleOutcome {
            intent_effect,
            runtime_effect: runtime_outcome.effect,
            observed: runtime_outcome.observed,
            diagnostics: runtime_outcome.diagnostics,
        };

        let ctx = EventContext::from_outcome(
            OperationKind::Remove,
            generation,
            &outcome,
            marker_was_incomplete,
            None,
        );
        for e in events::outcome_events(task_id, &ctx) {
            self.publish_completion_or_infra(task_id, &e).await;
        }

        Ok(outcome)
    }

    /// Completion events (`Stopped`/`Removed`) commit the fact + clear the
    /// marker atomically; infrastructure events (`Initializing`) just publish.
    async fn publish_completion_or_infra(&self, task_id: &str, event: &LifecycleEvent) {
        use super::events::EventFact;
        match event.fact {
            EventFact::Stopped | EventFact::Removed => {
                // The transactional-outbox commit: persist the event + clear
                // the marker in one DB transaction, then publish.
                if let Err(e) = self.store.commit_completion(task_id, &event.event_id).await {
                    tracing::warn!(task_id = %task_id, error = %e, "failed to commit completion fact");
                }
                self.events.publish(task_id, event).await;
            }
            _ => self.events.publish(task_id, event).await,
        }
    }

    async fn clear_marker_best_effort(&self, task_id: &str) {
        if let Err(e) = self.store.clear_marker(task_id).await {
            tracing::warn!(task_id = %task_id, error = %e, "failed to clear marker");
        }
    }
}

// re-export for use by finish signatures without a leading `super::`.
use super::runtime::RuntimeOutcome as _RuntimeOutcome;
#[allow(unused_imports)]
use _RuntimeOutcome as _KeepRuntimeOutcomeInScope;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::sandbox::lifecycle::error::RuntimeError;
    use crate::sandbox::lifecycle::runtime::RuntimeOutcome;
    use chrono::TimeZone;

    // ---- In-memory / recording doubles ----

    fn fixed_now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).single().expect("valid ts")
    }

    struct TestClock;
    impl Clock for TestClock {
        fn now(&self) -> DateTime<Utc> {
            fixed_now()
        }
    }

    /// Shared ordered log so tests can assert call ordering across ports.
    type Log = Arc<Mutex<Vec<String>>>;

    struct InMemoryStore {
        intent: Mutex<Option<TaskIntent>>,
        next_gen: Mutex<i64>,
        commits: Mutex<Vec<String>>,
        marker_cleared: Mutex<bool>,
        fail_persist: bool,
    }

    impl InMemoryStore {
        fn with_intent(intent: TaskIntent) -> Arc<Self> {
            Arc::new(Self {
                intent: Mutex::new(Some(intent)),
                next_gen: Mutex::new(1),
                commits: Mutex::new(vec![]),
                marker_cleared: Mutex::new(false),
                fail_persist: false,
            })
        }
        fn missing() -> Arc<Self> {
            Arc::new(Self {
                intent: Mutex::new(None),
                next_gen: Mutex::new(1),
                commits: Mutex::new(vec![]),
                marker_cleared: Mutex::new(false),
                fail_persist: false,
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
    impl LifecycleStore for InMemoryStore {
        async fn load(&self, _task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
            Ok(self.intent.lock().expect("lock").clone())
        }
        async fn allocate_generation(&self, _task_id: &str) -> Result<OperationGeneration, StoreError> {
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
                return Err(StoreError("persist failed".into()));
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
        async fn commit_completion(&self, _task_id: &str, event_id: &str) -> Result<(), StoreError> {
            self.commits.lock().expect("lock").push(event_id.to_string());
            // Committing a completion clears the marker atomically.
            if let Some(cur) = self.intent.lock().expect("lock").as_mut() {
                cur.marker = None;
            }
            Ok(())
        }
        async fn clear_marker(&self, _task_id: &str) -> Result<(), StoreError> {
            *self.marker_cleared.lock().expect("lock") = true;
            if let Some(cur) = self.intent.lock().expect("lock").as_mut() {
                cur.marker = None;
            }
            Ok(())
        }
    }

    struct RecordingAgentStop {
        log: Log,
    }
    #[async_trait]
    impl AgentStopPort for RecordingAgentStop {
        async fn stop_agent(&self, _task_id: &str) {
            self.log.lock().expect("lock").push("stop_agent".into());
        }
    }

    struct RecordingEvents {
        published: Mutex<Vec<LifecycleEvent>>,
    }
    impl RecordingEvents {
        fn new() -> Arc<Self> {
            Arc::new(Self { published: Mutex::new(vec![]) })
        }
        fn facts(&self) -> Vec<super::super::events::EventFact> {
            self.published.lock().expect("lock").iter().map(|e| e.fact).collect()
        }
        fn ids(&self) -> Vec<String> {
            self.published.lock().expect("lock").iter().map(|e| e.event_id.clone()).collect()
        }
    }
    #[async_trait]
    impl LifecycleEventSink for RecordingEvents {
        async fn publish(&self, _task_id: &str, event: &LifecycleEvent) {
            self.published.lock().expect("lock").push(event.clone());
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
        fn take(slot: &Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>) -> Result<RuntimeOutcome, RuntimeError> {
            slot.lock()
                .expect("lock")
                .take()
                .unwrap_or_else(|| Ok(RuntimeOutcome::no_op(ObservedStatus::Ready)))
        }
    }
    #[async_trait]
    impl SandboxRuntime for RecordingRuntime {
        async fn ensure_running(&self, _t: &str, ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
            self.log.lock().expect("lock").push("ensure_running".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.ensure)
        }
        async fn stop(&self, _t: &str, ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
            self.log.lock().expect("lock").push("runtime_stop".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.stop_res)
        }
        async fn restart(&self, _t: &str, ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
            self.log.lock().expect("lock").push("runtime_restart".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.restart_res)
        }
        async fn redeploy(&self, _t: &str, ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
            self.log.lock().expect("lock").push("runtime_redeploy".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.redeploy_res)
        }
        async fn remove(&self, _t: &str, _r: ArchiveRetention, ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
            self.log.lock().expect("lock").push("runtime_remove".into());
            *self.last_ctx.lock().expect("lock") = Some(ctx);
            Self::take(&self.remove_res)
        }
    }

    struct TestGuard;
    impl LockGuard for TestGuard {}

    /// A lock that grants unless pre-loaded with a conflicting holder.
    struct InMemoryLock {
        held_by: Mutex<Option<OperationKind>>,
    }
    impl InMemoryLock {
        fn free() -> Arc<Self> {
            Arc::new(Self { held_by: Mutex::new(None) })
        }
        fn held(kind: OperationKind) -> Arc<Self> {
            Arc::new(Self { held_by: Mutex::new(Some(kind)) })
        }
    }
    #[async_trait]
    impl LifecycleLock for InMemoryLock {
        async fn try_acquire(&self, _t: &str, _kind: OperationKind) -> Result<Box<dyn LockGuard>, OperationKind> {
            match *self.held_by.lock().expect("lock") {
                Some(holder) => Err(holder),
                None => Ok(Box::new(TestGuard)),
            }
        }
    }

    // ---- Builders ----

    fn open_running() -> TaskIntent {
        TaskIntent {
            status: TaskStatus::Open,
            desired_state: SandboxDesiredState::Running,
            stop_reason: None,
            retention: None,
            marker: None,
            desired_state_updated_at: fixed_now(),
        }
    }
    fn open_stopped() -> TaskIntent {
        TaskIntent {
            status: TaskStatus::Open,
            desired_state: SandboxDesiredState::Stopped,
            stop_reason: Some(SandboxStopReason::User),
            retention: None,
            marker: None,
            desired_state_updated_at: fixed_now(),
        }
    }
    fn closed() -> TaskIntent {
        TaskIntent {
            status: TaskStatus::Closed,
            desired_state: SandboxDesiredState::Removed,
            stop_reason: Some(SandboxStopReason::Cleanup),
            retention: Some(ArchiveRetention::RetainArchives),
            marker: None,
            desired_state_updated_at: fixed_now(),
        }
    }

    struct Harness {
        lifecycle: Lifecycle,
        store: Arc<InMemoryStore>,
        runtime: Arc<RecordingRuntime>,
        events: Arc<RecordingEvents>,
        log: Log,
    }

    fn harness(intent: Option<TaskIntent>, lock: Arc<InMemoryLock>) -> Harness {
        let log: Log = Arc::new(Mutex::new(vec![]));
        let store = match intent {
            Some(i) => InMemoryStore::with_intent(i),
            None => InMemoryStore::missing(),
        };
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
        Harness { lifecycle, store, runtime, events, log }
    }

    // ============================ Tests ============================

    // ---- Eligibility matrix ----

    #[tokio::test]
    async fn closed_task_rejects_start_restart_redeploy() {
        for op in ["start", "restart", "redeploy"] {
            let h = harness(Some(closed()), InMemoryLock::free());
            let res = match op {
                "start" => h.lifecycle.start("t").await,
                "restart" => h.lifecycle.restart("t").await,
                _ => h.lifecycle.redeploy("t").await,
            };
            assert!(matches!(res, Err(LifecycleError::NotAllowed { .. })), "{op}");
            // No runtime effect on a rejected op.
            assert!(h.log.lock().expect("lock").is_empty(), "{op}");
        }
    }

    #[tokio::test]
    async fn closed_task_permits_idempotent_stop_and_remove() {
        let h = harness(Some(closed()), InMemoryLock::free());
        h.runtime.stop_res.lock().expect("lock").replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Stopped)));
        assert!(h.lifecycle.stop("t", SandboxStopReason::User).await.is_ok());

        let h = harness(Some(closed()), InMemoryLock::free());
        h.runtime.remove_res.lock().expect("lock").replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Removed)));
        assert!(h.lifecycle.remove("t", ArchiveRetention::DeleteArchives).await.is_ok());
    }

    #[tokio::test]
    async fn missing_task_is_not_found() {
        let h = harness(None, InMemoryLock::free());
        assert!(matches!(h.lifecycle.start("t").await, Err(LifecycleError::TaskNotFound(_))));
    }

    // ---- Intent-before-effect ordering ----

    #[tokio::test]
    async fn start_persists_running_intent_before_runtime_effect() {
        let h = harness(Some(open_stopped()), InMemoryLock::free());
        h.runtime.ensure.lock().expect("lock").replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)));
        let out = h.lifecycle.start("t").await.expect("ok");
        // Intent moved to Running, reason cleared.
        let cur = h.store.current().expect("intent");
        assert_eq!(cur.desired_state, SandboxDesiredState::Running);
        assert_eq!(cur.stop_reason, None);
        assert_eq!(out.observed, ObservedStatus::Starting);
        assert!(out.intent_effect.changed());
        assert!(out.runtime_effect.applied());
    }

    #[tokio::test]
    async fn intent_write_failure_yields_no_runtime_effect() {
        let log: Log = Arc::new(Mutex::new(vec![]));
        let store = Arc::new(InMemoryStore {
            intent: Mutex::new(Some(open_stopped())),
            next_gen: Mutex::new(1),
            commits: Mutex::new(vec![]),
            marker_cleared: Mutex::new(false),
            fail_persist: true,
        });
        let runtime = Arc::new(RecordingRuntime::new(log.clone()));
        let lifecycle = Lifecycle::new(
            store,
            runtime,
            Arc::new(RecordingAgentStop { log: log.clone() }),
            RecordingEvents::new(),
            InMemoryLock::free(),
            Arc::new(TestClock),
        );
        let res = lifecycle.start("t").await;
        assert!(matches!(res, Err(LifecycleError::IntentWrite { .. })));
        // Runtime never invoked.
        assert!(log.lock().expect("lock").is_empty());
    }

    // ---- Graceful stop ordering ----

    #[tokio::test]
    async fn stop_issues_agent_stop_before_runtime_effect() {
        let h = harness(Some(open_running()), InMemoryLock::free());
        h.runtime.stop_res.lock().expect("lock").replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
        h.lifecycle.stop("t", SandboxStopReason::User).await.expect("ok");
        let log = h.log.lock().expect("lock").clone();
        let stop_agent = log.iter().position(|s| s == "stop_agent").expect("stop_agent");
        let runtime_stop = log.iter().position(|s| s == "runtime_stop").expect("runtime_stop");
        assert!(stop_agent < runtime_stop, "StopAgent must precede runtime stop: {log:?}");
    }

    #[tokio::test]
    async fn remove_issues_agent_stop_before_runtime_effect() {
        let h = harness(Some(open_running()), InMemoryLock::free());
        h.runtime.remove_res.lock().expect("lock").replace(Ok(RuntimeOutcome::applied(ObservedStatus::Removed)));
        h.lifecycle.remove("t", ArchiveRetention::DeleteArchives).await.expect("ok");
        let log = h.log.lock().expect("lock").clone();
        let stop_agent = log.iter().position(|s| s == "stop_agent").expect("stop_agent");
        let runtime_remove = log.iter().position(|s| s == "runtime_remove").expect("runtime_remove");
        assert!(stop_agent < runtime_remove, "StopAgent must precede runtime remove: {log:?}");
    }

    // ---- No-op suppression ----

    #[tokio::test]
    async fn reusing_ready_sandbox_is_no_op_and_publishes_nothing() {
        let h = harness(Some(open_running()), InMemoryLock::free());
        h.runtime.ensure.lock().expect("lock").replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Ready)));
        let out = h.lifecycle.start("t").await.expect("ok");
        assert!(out.is_pure_no_op());
        assert!(h.events.facts().is_empty());
        // Marker cleared synchronously.
        assert!(h.store.marker_cleared());
    }

    // ---- Start matrix (representative) ----

    #[tokio::test]
    async fn start_ready_synchronously_docker_style() {
        // Docker completes synchronously and reports Ready with Applied.
        let h = harness(Some(open_stopped()), InMemoryLock::free());
        h.runtime.ensure.lock().expect("lock").replace(Ok(RuntimeOutcome::applied(ObservedStatus::Ready)));
        let out = h.lifecycle.start("t").await.expect("ok");
        assert_eq!(out.observed, ObservedStatus::Ready);
        // Applied start publishes Initializing (infra), never Ready.
        assert!(h.events.facts().contains(&super::super::events::EventFact::Initializing));
        assert!(h.events.published.lock().expect("lock").iter().all(|e| e.status != moatless_events::SandboxStatus::Ready));
    }

    #[tokio::test]
    async fn start_runtime_failure_publishes_one_error_and_keeps_intent() {
        let h = harness(Some(open_stopped()), InMemoryLock::free());
        h.runtime.ensure.lock().expect("lock").replace(Err(RuntimeError::Infrastructure("timeout".into())));
        let res = h.lifecycle.start("t").await;
        assert!(matches!(res, Err(LifecycleError::Runtime { .. })));
        // Intent still Running (target survives the failure).
        assert_eq!(h.store.current().expect("intent").desired_state, SandboxDesiredState::Running);
        // Exactly one Error fact.
        assert_eq!(h.events.facts(), vec![super::super::events::EventFact::Error]);
    }

    // ---- Correlation: generation reaches the adapter ----

    #[tokio::test]
    async fn generation_reaches_the_runtime_context() {
        let h = harness(Some(open_stopped()), InMemoryLock::free());
        h.runtime.ensure.lock().expect("lock").replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)));
        h.lifecycle.start("t").await.expect("ok");
        let ctx = h.runtime.last_ctx.lock().expect("lock").expect("ctx");
        // First allocated generation is 1.
        assert_eq!(ctx.operation_generation, OperationGeneration(1));
    }

    // ---- Race matrix (contention) ----

    #[tokio::test]
    async fn contended_lock_surfaces_operation_in_progress_with_holder_kind() {
        let h = harness(Some(open_running()), InMemoryLock::held(OperationKind::Stop));
        let res = h.lifecycle.start("t").await;
        match res {
            Err(LifecycleError::OperationInProgress { kind }) => assert_eq!(kind, OperationKind::Stop),
            other => panic!("expected OperationInProgress, got {other:?}"),
        }
        // No runtime effect while contended.
        assert!(h.log.lock().expect("lock").is_empty());
    }

    // ---- Stop completion + durability ----

    #[tokio::test]
    async fn stop_against_running_publishes_stopped_and_commits_completion() {
        let h = harness(Some(open_running()), InMemoryLock::free());
        h.runtime.stop_res.lock().expect("lock").replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
        h.lifecycle.stop("t", SandboxStopReason::IdleReaper).await.expect("ok");
        assert!(h.events.facts().contains(&super::super::events::EventFact::Stopped));
        // Completion fact committed atomically (marker cleared).
        assert_eq!(h.store.commits(), vec!["sandbox:t:1:stopped".to_string()]);
        assert!(h.store.current().expect("intent").marker.is_none());
    }

    #[tokio::test]
    async fn stop_against_already_stopped_is_changed_noop_still_publishes() {
        // Reason changes (User → IdleReaper) so intent is Changed; compute is
        // already stopped so runtime is NoOp. The stop is still real.
        let h = harness(Some(open_stopped()), InMemoryLock::free());
        h.runtime.stop_res.lock().expect("lock").replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Stopped)));
        let out = h.lifecycle.stop("t", SandboxStopReason::IdleReaper).await.expect("ok");
        assert!(out.intent_effect.changed());
        assert!(!out.runtime_effect.applied());
        assert!(h.events.facts().contains(&super::super::events::EventFact::Stopped));
    }

    // ---- Remove: acceptance + completion, strict success ----

    #[tokio::test]
    async fn successful_remove_emits_removing_then_removed_and_commits() {
        let h = harness(Some(open_running()), InMemoryLock::free());
        h.runtime.remove_res.lock().expect("lock").replace(Ok(RuntimeOutcome::applied(ObservedStatus::Removed)));
        h.lifecycle.remove("t", ArchiveRetention::DeleteArchives).await.expect("ok");
        assert_eq!(
            h.events.facts(),
            vec![super::super::events::EventFact::Removing, super::super::events::EventFact::Removed]
        );
        // Retention persisted with the Removed intent.
        assert_eq!(h.store.current().expect("intent").retention, Some(ArchiveRetention::DeleteArchives));
        assert_eq!(h.store.commits(), vec!["sandbox:t:1:removed".to_string()]);
    }

    #[tokio::test]
    async fn failed_remove_keeps_removed_intent_and_publishes_error() {
        // Strict removal: an archive/absence failure is not success.
        let h = harness(Some(open_running()), InMemoryLock::free());
        h.runtime.remove_res.lock().expect("lock").replace(Err(RuntimeError::NotAbsent("pvc".into())));
        let res = h.lifecycle.remove("t", ArchiveRetention::DeleteArchives).await;
        assert!(matches!(res, Err(LifecycleError::Runtime { .. })));
        // Intent stays Removed with its policy for the reconciler to retry.
        let cur = h.store.current().expect("intent");
        assert_eq!(cur.desired_state, SandboxDesiredState::Removed);
        assert_eq!(cur.retention, Some(ArchiveRetention::DeleteArchives));
        // Removing acceptance + Error, but NOT Removed completion.
        let facts = h.events.facts();
        assert!(facts.contains(&super::super::events::EventFact::Removing));
        assert!(facts.contains(&super::super::events::EventFact::Error));
        assert!(!facts.contains(&super::super::events::EventFact::Removed));
        // No completion committed.
        assert!(h.store.commits().is_empty());
    }

    // ---- Deterministic event ids dedup across a generation ----

    #[tokio::test]
    async fn error_event_id_is_generation_scoped() {
        let h = harness(Some(open_stopped()), InMemoryLock::free());
        h.runtime.ensure.lock().expect("lock").replace(Err(RuntimeError::Infrastructure("x".into())));
        let _ = h.lifecycle.start("t").await;
        assert_eq!(h.events.ids(), vec!["sandbox:t:1:error".to_string()]);
    }
}
