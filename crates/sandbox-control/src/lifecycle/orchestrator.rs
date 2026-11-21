//! The `Lifecycle` orchestrator and its ports.
//!
//! Resolves intent, enforces the Closed guard, allocates or reuses an operation
//! generation, and finalizes through one completion path: facts persist and the
//! marker clears in one transaction before the event publishes.
//!
//! Multi-step transitions run detached from the caller future, so a caller
//! disconnect cannot cancel a half-applied transition — see
//! [`Lifecycle::detached_span`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::Instrument;

use moatless_ports::sandbox as core_sandbox;

use moatless_ports::sandbox::{AgentStopOutcome, SandboxLifecycle, SandboxOpError};
use moatless_sandbox_runtime::error::SandboxError;
use moatless_types::access_control::ScopedResource;
use moatless_types::sandbox::{SandboxDesiredState, SandboxStopReason};
pub use moatless_types::store::StoreError;
use moatless_types::task::TaskStatus;
use moatless_types::task::Visibility;

use super::dao::{IntentWrite, LifecycleDao};
use super::error::LifecycleError;
use super::events::{self, EventContext, FailureDetail, LifecycleEvent};
use moatless_sandbox_runtime::observation_events::EventFact;
use moatless_sandbox_runtime::operation::{
    CorrelationMarker, IntentEffect, ObservedStatus, OperationGeneration, OperationKind,
    RuntimeEffect,
};
use moatless_sandbox_runtime::outcome::LifecycleOutcome;
use moatless_sandbox_runtime::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
use moatless_types::sandbox::ArchiveRetention;

/// Persisted lifecycle intent for one Task's Sandbox membership.
///
/// Answers who may reach it, what the Task's status permits, and what state it
/// desires; `owner_user_id` is the Sandbox's own, `visibility` is the Task's.
#[derive(Debug, Clone)]
pub struct TaskIntent {
    pub status: TaskStatus,
    pub visibility: Visibility,
    pub owner_user_id: Option<String>,
    /// The Task's Reporter, who reads and writes the Sandbox without owning it.
    pub reporter_user_id: Option<String>,
    pub sandbox_id: String,
    pub workspace_id: String,
    /// Whether the Sandbox runs without a person watching it; see
    /// `SANDBOX_HEADLESS`.
    pub headless: bool,
    pub desired_state: SandboxDesiredState,
    pub stop_reason: Option<SandboxStopReason>,
    /// Retention persisted alongside a `Removed` intent so a crashed remove is
    /// retried with the original policy.
    pub retention: Option<ArchiveRetention>,
    pub marker: Option<CorrelationMarker>,
    pub desired_state_updated_at: DateTime<Utc>,
}

impl TaskIntent {
    /// The Sandbox as an access-control subject.
    pub fn scoped_resource(&self) -> ScopedResource {
        ScopedResource::task(self.owner_user_id.as_deref())
            .reported_by(self.reporter_user_id.as_deref())
    }
}

/// One `moat cmd` command a pod says still owes its agent an outcome.
///
/// Metadata only, unread here — kept so a replacement pod can name what died;
/// `state` stays a string since this band never branches on it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutstandingCommand {
    pub id: String,
    pub command: String,
    pub label: String,
    pub started_at_unix_ms: i64,
    pub state: String,
    pub exit_code: Option<i32>,
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

/// The narrow internal agent-stop port. Graceful stop is lifecycle's ordering
/// rule: `stop`/`remove` issue a best-effort `StopAgent` **before** the runtime
/// effect so the pod exits promptly. Failures never block the transition.
#[async_trait]
pub trait AgentStopPort: Send + Sync {
    /// `reason` is forwarded to the sandbox so the terminal `agent.stopped`
    /// event attributes the stop to its initiator (user, idle reaper, ...).
    async fn stop_agent(&self, task_id: &str, reason: SandboxStopReason) -> AgentStopOutcome;
}

/// The narrow internal workspace-sync port.
///
/// Lifecycle asks for a push to S3 before the runtime effect on
/// `stop`/`remove`, since the sandbox alone can read its volume before it's
/// unreachable or deleted; failures never block the transition.
#[async_trait]
pub trait WorkspaceSyncPort: Send + Sync {
    async fn sync_workspace(&self, task_id: &str);
}

/// The narrow internal workspace-archive port.
///
/// The cold transition trades a volume for a snapshot, so it may run only when
/// the snapshot is real and current: this port answers when the archive was
/// last written, and how to hand a detached volume a way to write a new one.
#[async_trait]
pub trait WorkspaceArchivePort: Send + Sync {
    /// When the task's workspace archive was last written.
    ///
    /// `Ok(None)` means no archive; `Err` means unanswered — a caller about to
    /// delete the only other copy must treat that as "not proven", never "no
    /// archive".
    async fn archived_at(&self, task_id: &str) -> Result<Option<DateTime<Utc>>, String>;

    /// A short-lived presigned URL that accepts one `PUT` of the task's
    /// workspace archive. Handed to whatever can still read the detached
    /// volume, so no long-lived credential leaves the backend.
    async fn presign_upload(&self, task_id: &str) -> Result<String, String>;
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
        diagnostics: &moatless_sandbox_runtime::outcome::Diagnostics,
    );
}

/// A held per-Task lock. Dropping it releases the lock.
pub(crate) trait LockGuard: Send {}

/// Per-Task serialization across backend replicas.
///
/// No lock-free fast path: Postgres uses `pg_try_advisory_lock` on a dedicated
/// pool with bounded retry, and on exhaustion the orchestrator surfaces
/// `OperationInProgress { kind }` naming the holder.
#[async_trait]
pub(crate) trait LifecycleLock: Send + Sync {
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
    workspace_sync: Option<Arc<dyn WorkspaceSyncPort>>,
    workspace_archive: Option<Arc<dyn WorkspaceArchivePort>>,
    events: Arc<dyn LifecycleEventSink>,
    lock: Arc<dyn LifecycleLock>,
    clock: Arc<dyn WallClock>,
    plan: Option<super::plan::ProvisionDeps>,
    default_sandbox_image: Option<String>,
    /// Bounded lock-acquire attempts before surfacing `OperationInProgress`.
    lock_attempts: u32,
}

impl Lifecycle {
    pub(crate) fn new(
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
            workspace_sync: None,
            workspace_archive: None,
            events,
            lock,
            clock,
            plan: None,
            default_sandbox_image: None,
            lock_attempts: 1,
        }
    }

    /// Attach the pre-stop workspace push. Optional: a deployment with no S3
    /// configured has nothing to push to, and leaving it unset skips the RPC
    /// entirely rather than issuing one the sandbox would decline.
    pub(crate) fn with_workspace_sync(
        mut self,
        workspace_sync: Arc<dyn WorkspaceSyncPort>,
    ) -> Self {
        self.workspace_sync = Some(workspace_sync);
        self
    }

    /// Attach the archive reader the cold transition proves itself against.
    ///
    /// Optional, like the sync port: with no S3 there is no snapshot, so
    /// leaving it unset disables the transition rather than deleting a volume
    /// with nowhere to fall back to.
    pub(crate) fn with_workspace_archive(mut self, archive: Arc<dyn WorkspaceArchivePort>) -> Self {
        self.workspace_archive = Some(archive);
        self
    }

    /// Make the durable copy current before an effect destroys the live one.
    /// Best-effort, exactly like [`AgentStopPort::stop_agent`].
    async fn sync_workspace_before_effect(&self, task_id: &str) {
        if let Some(sync) = self.workspace_sync.as_ref() {
            sync.sync_workspace(task_id).await;
        }
    }

    pub(crate) fn with_lock_attempts(mut self, attempts: u32) -> Self {
        self.lock_attempts = attempts.max(1);
        self
    }

    pub(crate) fn with_plan_knowledge(
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
            source: moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure(
                "sandbox plan knowledge unavailable".to_string(),
            ),
        })?;
        let default_image =
            self.default_sandbox_image
                .as_deref()
                .ok_or_else(|| LifecycleError::Runtime {
                    task_id: task_id.to_string(),
                    source: moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure(
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
        observation: moatless_sandbox_runtime::classify::Observation,
    ) -> Result<moatless_sandbox_runtime::classify::ClassificationResult, LifecycleError> {
        let intent = self.require_intent(task_id).await?;
        let suppression_active = intent.marker.is_some_and(|marker| {
            moatless_sandbox_runtime::marker::edge_suppression_active(
                &marker,
                self.clock.now(),
                moatless_sandbox_runtime::marker::EDGE_SUPPRESSION_STALE_AFTER,
            )
        });
        Ok(
            moatless_sandbox_runtime::classify::classify_with_correlation(
                observation,
                intent.desired_state,
                intent.marker,
                suppression_active,
            ),
        )
    }

    // ---- Public operations (detached from the caller future) ----

    /// The span a detached operation runs under: a root of its own, linked
    /// back to whoever asked for it.
    ///
    /// Must not reuse the caller's span: `tracing-opentelemetry` restamps a
    /// span's end time on every exit, so the caller would report as having
    /// run as long as the whole detached operation.
    fn detached_span(op: &'static str, task_id: &str) -> tracing::Span {
        let span =
            tracing::info_span!(parent: None, "sandbox.lifecycle", op = %op, task_id = %task_id);
        span.follows_from(tracing::Span::current());
        span
    }

    pub async fn start(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let me = self.clone();
        let id = task_id.to_string();
        let span = Self::detached_span("start", task_id);
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
        let span = Self::detached_span("stop", task_id);
        Self::join(
            task_id.to_string(),
            tokio::spawn(async move { me.op_stop(id, reason).await }.instrument(span)),
        )
        .await
    }

    pub async fn restart(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let me = self.clone();
        let id = task_id.to_string();
        let span = Self::detached_span("restart", task_id);
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
        let span = Self::detached_span("redeploy", task_id);
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
        let span = Self::detached_span("remove", task_id);
        Self::join(
            task_id.to_string(),
            tokio::spawn(async move { me.op_remove(id, retention).await }.instrument(span)),
        )
        .await
    }

    /// Reclaims a long-stopped Sandbox's volume into its S3 snapshot; desired
    /// state stays `Stopped`, since cold is not a fourth state. See
    /// `op_archive_storage` for the deletion proof it requires.
    pub async fn archive_storage(&self, task_id: &str) -> Result<LifecycleOutcome, LifecycleError> {
        let me = self.clone();
        let id = task_id.to_string();
        let span = Self::detached_span("archive_storage", task_id);
        Self::join(
            task_id.to_string(),
            tokio::spawn(async move { me.op_archive_storage(id).await }.instrument(span)),
        )
        .await
    }

    /// Which of `task_ids` are stopped and still own storage, and when each
    /// stopped.
    ///
    /// A read, so no lock and no detachment; answers only what storage exists
    /// and leaves the retention clock to the caller.
    pub async fn stopped_with_storage(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, LifecycleError> {
        self.store
            .stopped_with_storage(task_ids)
            .await
            .map(|rows| rows.into_iter().collect())
            .map_err(|e| LifecycleError::IntentLoad {
                // A batch read has no single Task to name; the count is what
                // makes the failure legible in a log line.
                task_id: format!("<{} ids>", task_ids.len()),
                reason: e.to_string(),
            })
    }

    /// The persisted desired state for the Task's Sandbox.
    ///
    /// Absence of a row reads as `Stopped` (never requested = converged
    /// Stopped); a missing Task is `TaskNotFound`.
    pub async fn desired_state(
        &self,
        task_id: &str,
    ) -> Result<SandboxDesiredState, LifecycleError> {
        Ok(self.require_intent(task_id).await?.desired_state)
    }

    /// The Task's Sandbox membership as one durable read: its access-control
    /// pair, its Task's status and visibility, and its desired state. The HTTP
    /// handlers authorize, guard, and observe off this single value.
    pub async fn intent(&self, task_id: &str) -> Result<TaskIntent, LifecycleError> {
        self.require_intent(task_id).await
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
        let intent = self.require_intent(&task_id).await?;
        let _guard = self
            .acquire(&intent.sandbox_id, OperationKind::Start)
            .await?;
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
        let intent = self.require_intent(&task_id).await?;
        let _guard = self.acquire(&intent.sandbox_id, kind).await?;
        Self::guard_toward_running(&task_id, kind, &intent)?;
        self.run_toward_running(&task_id, kind, &intent).await
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id, op = "stop"))]
    async fn op_stop(
        self,
        task_id: String,
        reason: SandboxStopReason,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let intent = self.require_intent(&task_id).await?;
        let _guard = self
            .acquire(&intent.sandbox_id, OperationKind::Stop)
            .await?;
        // stop is permitted in every status (idempotent for Closed).
        self.run_stop(&task_id, &intent, reason).await
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id, op = "remove"))]
    async fn op_remove(
        self,
        task_id: String,
        retention: ArchiveRetention,
    ) -> Result<LifecycleOutcome, LifecycleError> {
        let intent = self.require_intent(&task_id).await?;
        let _guard = self
            .acquire(&intent.sandbox_id, OperationKind::Remove)
            .await?;
        // remove is permitted in every status.
        self.run_remove(&task_id, &intent, retention).await
    }

    /// The cold transition.
    ///
    /// Unlike every other operation here, it writes no intent and publishes
    /// no events, since the Sandbox is already `Stopped` and stays so. It
    /// owns proof instead: the volume is deleted only once the archive is
    /// known at least as new as the stop (or the runtime can write one).
    #[tracing::instrument(skip_all, fields(task_id = %task_id, op = "archive_storage"))]
    async fn op_archive_storage(self, task_id: String) -> Result<LifecycleOutcome, LifecycleError> {
        // Held under `Stop`: this is a continuation of the stop that preceded
        // it, and reusing the kind keeps it mutually exclusive with every other
        // operation without adding a variant that the event, marker, classify
        // and reconciler paths would each have to learn.
        let intent = self.require_intent(&task_id).await?;
        let _guard = self
            .acquire(&intent.sandbox_id, OperationKind::Stop)
            .await?;

        // Intent is re-read under the lock on purpose: the caller decided this
        // Task was cold from a snapshot taken before the lock, and a start in
        // between makes the volume live again.
        if intent.desired_state != SandboxDesiredState::Stopped
            || intent.status == TaskStatus::Closed
        {
            moatless_sandbox_runtime::metrics::record_storage_reclaim("skipped");
            return Ok(Self::reclaim_noop());
        }

        // No archive port means no S3, which means the volume is the only copy
        // — there is nothing to reclaim toward.
        let Some(archive) = self.workspace_archive.clone() else {
            moatless_sandbox_runtime::metrics::record_storage_reclaim("skipped");
            return Ok(Self::reclaim_noop());
        };

        // `run_stop` persists the Stopped intent before pushing the workspace,
        // so an archive from that stop is strictly newer than this timestamp.
        // The two clocks are different machines' (Postgres and the object
        // store), but skew only ever costs a needless rescue, never a wrongly
        // dropped volume.
        let stopped_at = intent.desired_state_updated_at;
        let archived_at = match archive.archived_at(&task_id).await {
            Ok(at) => at,
            Err(reason) => {
                moatless_sandbox_runtime::metrics::record_storage_reclaim("blocked");
                return Err(LifecycleError::ArchiveUnproven { task_id, reason });
            }
        };

        let rescue_url = match archived_at {
            Some(at) if at > stopped_at => {
                tracing::info!(%at, %stopped_at, "workspace archive is current, reclaiming volume");
                None
            }
            other => {
                tracing::info!(
                    archived_at = ?other,
                    %stopped_at,
                    "workspace archive is stale or absent, rescuing volume before reclaim"
                );
                match archive.presign_upload(&task_id).await {
                    Ok(url) => Some(url),
                    Err(reason) => {
                        moatless_sandbox_runtime::metrics::record_storage_reclaim("blocked");
                        return Err(LifecycleError::ArchiveUnproven { task_id, reason });
                    }
                }
            }
        };
        let rescuing = rescue_url.is_some();

        // A fresh generation, as for any operation that mutates runtime
        // resources: it labels the rescue workload and correlates the deletions
        // that follow it.
        let ctx = RuntimeOperationContext::new(self.allocate(&intent.sandbox_id).await?);
        match self
            .runtime
            .reclaim_storage(&intent.sandbox_id, rescue_url.as_deref(), ctx)
            .await
        {
            Ok(outcome) => {
                moatless_sandbox_runtime::metrics::record_storage_reclaim(
                    match (outcome.effect, rescuing) {
                        // Nothing was there to reclaim — a task cooled by an earlier
                        // pass, or one that never had a volume.
                        (RuntimeEffect::NoOp, _) => "skipped",
                        (_, true) => "rescued",
                        (_, false) => "reclaimed",
                    },
                );
                // Storage is now provably absent (the runtime proves absence
                // before returning, and `NoOp` means it was already gone), so
                // record it and stop offering this Task to the cold branch. A
                // failed mark only costs one idempotent retry next cycle.
                if let Err(e) = self.store.mark_storage_reclaimed(&intent.sandbox_id).await {
                    tracing::warn!(error = %e, "could not record the storage reclaim");
                }
                Ok(LifecycleOutcome::new(
                    IntentEffect::Unchanged,
                    outcome.effect,
                    outcome.observed,
                ))
            }
            Err(source) => {
                // The runtime is strict about absence, so a failure here leaves
                // the volume — the next cycle retries against the same intent.
                moatless_sandbox_runtime::metrics::record_storage_reclaim("blocked");
                Err(LifecycleError::Runtime { task_id, source })
            }
        }
    }

    /// The cold transition's "nothing to do" answer: intent untouched, no
    /// runtime effect, and the Stopped status the Task already had.
    fn reclaim_noop() -> LifecycleOutcome {
        LifecycleOutcome::new(
            IntentEffect::Unchanged,
            RuntimeEffect::NoOp,
            ObservedStatus::Stopped,
        )
    }

    fn member_detach_noop(intent: &TaskIntent) -> LifecycleOutcome {
        let observed = match intent.desired_state {
            SandboxDesiredState::Running => ObservedStatus::Ready,
            SandboxDesiredState::Stopped => ObservedStatus::Stopped,
            SandboxDesiredState::Removed => ObservedStatus::Removed,
        };
        LifecycleOutcome::new(IntentEffect::Unchanged, RuntimeEffect::NoOp, observed)
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

    /// A recovery — a level op whose target matches persisted intent and whose
    /// marker is the same kind — reuses the persisted generation so attempt
    /// identity and event ids stay stable across retries; the returned bool
    /// marks whether an incomplete marker preceded this attempt. Edges always
    /// allocate fresh (they deliberately supersede).
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
            .resolve_generation(&intent.sandbox_id, kind, intent, target)
            .await?;
        let marker = CorrelationMarker::new(kind, generation, self.clock.now());
        let write = IntentWrite {
            desired_state: target,
            stop_reason: None,
            retention: None,
            marker,
        };
        let intent_effect = match self.persist(&intent.sandbox_id, &write).await {
            Ok(effect) => effect,
            Err(e) => return Err(self.publish_failure(task_id, kind, generation, e).await),
        };

        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = match kind {
            OperationKind::Start => self.runtime.ensure_running(&intent.sandbox_id, ctx).await,
            OperationKind::Restart => self.runtime.restart(&intent.sandbox_id, ctx).await,
            OperationKind::Redeploy => self.runtime.redeploy(&intent.sandbox_id, ctx).await,
            OperationKind::Stop | OperationKind::Remove => {
                unreachable!("toward-running kinds only")
            }
        };

        self.finalize(
            task_id,
            &intent.sandbox_id,
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
            .resolve_generation(&intent.sandbox_id, OperationKind::Stop, intent, target)
            .await?;
        let marker = CorrelationMarker::new(OperationKind::Stop, generation, self.clock.now());
        let write = IntentWrite {
            desired_state: target,
            stop_reason: Some(reason),
            retention: None,
            marker,
        };
        let intent_effect = match self.persist(&intent.sandbox_id, &write).await {
            Ok(effect) => effect,
            Err(e) => {
                return Err(self
                    .publish_failure(task_id, OperationKind::Stop, generation, e)
                    .await);
            }
        };

        // Graceful agent stop before the runtime effect (best-effort), then
        // push the workspace to S3 while the sandbox can still read it: the
        // stop comes first so the tar is taken against a tree the agent is
        // no longer writing to.
        self.agent_stop.stop_agent(task_id, reason).await;
        self.sync_workspace_before_effect(task_id).await;

        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = self.runtime.stop(&intent.sandbox_id, ctx).await;

        self.finalize(
            task_id,
            &intent.sandbox_id,
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
        if self
            .store
            .has_other_active_members(&intent.sandbox_id, task_id)
            .await
            .map_err(|e| LifecycleError::IntentLoad {
                task_id: task_id.to_string(),
                reason: e.to_string(),
            })?
        {
            self.agent_stop
                .stop_agent(task_id, SandboxStopReason::Cleanup)
                .await;
            return Ok(Self::member_detach_noop(intent));
        }

        let target = SandboxDesiredState::Removed;
        let (generation, marker_was_incomplete) = self
            .resolve_generation(&intent.sandbox_id, OperationKind::Remove, intent, target)
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
        let intent_effect = match self.persist(&intent.sandbox_id, &write).await {
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

        // Push the workspace only when the archives are being kept. A hard
        // delete is on its way to erasing the very object this would write, so
        // spending the stop budget on it buys nothing.
        if retention.retains() {
            self.sync_workspace_before_effect(task_id).await;
        }

        let ctx = RuntimeOperationContext::new(generation);
        let runtime_result = self
            .runtime
            .remove(&intent.sandbox_id, retention, ctx)
            .await;

        self.finalize(
            task_id,
            &intent.sandbox_id,
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

    /// On failure, publishes the deterministic `Error` fact and returns the
    /// error (intent/marker survive). On success, persists the completion
    /// fact and clears the marker in one transaction before publishing, so
    /// a commit failure blocks publication; a no-op against an
    /// already-converged target clears its marker synchronously instead.
    async fn finalize(
        &self,
        task_id: &str,
        sandbox_id: &str,
        run: OperationRun,
        runtime_result: Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError>,
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
                // Persist the completion fact and clear the marker
                // atomically; propagate failure and don't publish the
                // completion (intent/marker survive, so the deterministic
                // id is never published unpersisted). The failure publishes
                // instead, or a never-committed completion leaves no event.
                if let Err(e) = self
                    .store
                    .commit_fact_and_clear_marker(sandbox_id, task_id, &event)
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
            self.clear_marker_best_effort(sandbox_id, run.generation)
                .await;
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
    /// Failures before generation resolution have no stable event identity — a
    /// random id defeats dedup, a shared one collides — so they stay caller-
    /// and log-only instead.
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

    async fn clear_marker_best_effort(&self, sandbox_id: &str, generation: OperationGeneration) {
        // A no-op-generation marker owes no completion fact; if the clear fails
        // the reconciler's marker acknowledgement is the backstop.
        if let Err(e) = self.store.clear_marker(sandbox_id, generation).await {
            tracing::warn!(sandbox_id = %sandbox_id, error = %e, "failed to clear no-op marker");
        }
    }

    /// Durably abandon a stale caller-owned edge: the deterministic `Error`
    /// fact and marker clear commit atomically, then the fact publishes.
    ///
    /// `kind` is the abandoned edge's own operation, so the published fact
    /// names it rather than a generic runtime-failure label.
    pub(crate) async fn abandon_edge(
        &self,
        task_id: &str,
        kind: OperationKind,
        generation: OperationGeneration,
    ) -> Result<(), LifecycleError> {
        let intent = self.require_intent(task_id).await?;
        let failure = FailureDetail::new(
            kind,
            "edge_abandoned",
            format!(
                "no replacement observed within {}s; the operation was abandoned",
                moatless_sandbox_runtime::marker::EDGE_ABANDON_AFTER.as_secs()
            ),
        );
        let event = events::error_event(task_id, generation, failure);
        self.store
            .commit_fact_and_clear_marker(&intent.sandbox_id, task_id, &event)
            .await
            .map_err(|e| LifecycleError::CompletionCommit {
                task_id: task_id.to_string(),
                reason: e.to_string(),
            })?;
        self.events.publish(task_id, &event).await;
        Ok(())
    }
}

// The durable-lifecycle ports of the sandbox runtime contract. `Lifecycle`
// is the sole writer of Sandbox intent; these hand a transport only the
// one capability it needs, not the whole orchestrator. A `LifecycleError`
// reaches the contract through this module's `SandboxError`, keeping the
// three client-visible outcomes' mapping.
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

#[async_trait]
impl core_sandbox::SandboxOperator for Lifecycle {
    async fn restart(&self, task_id: &str) -> Result<(), core_sandbox::SandboxError> {
        Self::restart(self, task_id)
            .await
            .map(|_| ())
            .map_err(|e| SandboxError::from(e).into_contract())
    }

    async fn redeploy(&self, task_id: &str) -> Result<(), core_sandbox::SandboxError> {
        Self::redeploy(self, task_id)
            .await
            .map(|_| ())
            .map_err(|e| SandboxError::from(e).into_contract())
    }

    async fn cleanup(&self, task_id: &str) -> Result<(), core_sandbox::SandboxError> {
        Self::remove(self, task_id, ArchiveRetention::DeleteArchives)
            .await
            .map(|_| ())
            .map_err(|e| SandboxError::from(e).into_contract())
    }

    async fn expected_image(&self, task_id: &str) -> Result<String, core_sandbox::SandboxError> {
        Self::expected_sandbox_image(self, task_id)
            .await
            .map_err(|e| SandboxError::from(e).into_contract())
    }
}

/// Classify a `LifecycleError` for the tasks-band lifecycle port, so the
/// tasks band names no lifecycle error.
///
/// The three client-visible outcomes (404/400/409) keep their shape, and
/// everything else becomes an internal failure with its chain intact.
/// `InProgress` (unlike T3's `SandboxError`) keeps lock contention a 409.
fn op_error(e: LifecycleError) -> SandboxOpError {
    use LifecycleError as LE;
    match &e {
        LE::TaskNotFound(id) => SandboxOpError::NotFound(id.clone()),
        LE::NotAllowed { .. } => SandboxOpError::NotAllowed(e.to_string()),
        LE::OperationInProgress { .. } => SandboxOpError::InProgress(e.to_string()),
        _ => SandboxOpError::Internal(e.to_string()),
    }
}

/// The tasks-band lifecycle port. The closer, reaper, HTTP handlers, and
/// service drive a Task's sandbox through this without naming a `sandbox` type;
/// these methods are the whole tasks-band cross-boundary surface.
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

    async fn stop_agent(&self, task_id: &str, reason: SandboxStopReason) -> AgentStopOutcome {
        // No lock and no generation: the agent stopping changes nothing this
        // orchestrator owns, so it cannot conflict with a transition in flight.
        self.agent_stop.stop_agent(task_id, reason).await
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

    async fn archive_storage(&self, task_id: &str) -> Result<(), SandboxOpError> {
        Self::archive_storage(self, task_id)
            .await
            .map(|_| ())
            .map_err(op_error)
    }

    async fn stopped_with_storage(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, SandboxOpError> {
        Self::stopped_with_storage(self, task_ids)
            .await
            .map_err(op_error)
    }

    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxOpError> {
        Self::desired_state(self, task_id).await.map_err(op_error)
    }
}

#[cfg(test)]
mod tests;
