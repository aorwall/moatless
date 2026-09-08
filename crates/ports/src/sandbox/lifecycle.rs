use async_trait::async_trait;
use chrono::DateTime;
use chrono::Utc;
use moatless_types::sandbox::ArchiveRetention;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::sandbox::SandboxError;
use moatless_types::sandbox::SandboxStopReason;
use std::collections::HashMap;

#[async_trait]
pub trait SandboxIntentReader: Send + Sync {
    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxError>;
}

#[async_trait]
pub trait SandboxStarter: Send + Sync {
    async fn start(&self, task_id: &str) -> Result<(), SandboxError>;
}

#[async_trait]
pub trait SandboxStopper: Send + Sync {
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxError>;
}

/// The lifecycle surface the T3 sandbox panel drives beside start and stop:
/// three transitions, plus the one read that decides whether to offer one of
/// them.
///
/// Deliberately not part of [`SandboxLifecycle`], which is sized to the tasks
/// band and preserves its 409 through [`SandboxOpError`]. These flatten through
/// [`SandboxError`] like [`SandboxStarter`] and [`SandboxStopper`] beside them,
/// because the T3 surface renders one refusal for every lifecycle failure.
#[async_trait]
pub trait SandboxOperator: Send + Sync {
    /// Replace the Task's sandbox pod, keeping its deployment and its workspace.
    async fn restart(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Rebuild the Task's sandbox deployment from current repository config,
    /// keeping its workspace volume. What picks up a new image.
    async fn redeploy(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Remove the Task's sandbox and delete its archives, leaving the Task.
    ///
    /// The retention is fixed rather than passed in: [`ArchiveRetention::DeleteArchives`]
    /// is what the REST cleanup route applies, and the panel offers no other.
    async fn cleanup(&self, task_id: &str) -> Result<(), SandboxError>;

    /// The image a fresh deploy would install: the per-repo `docker_image`
    /// override, else the configured default.
    ///
    /// A read, on a port of transitions, because it is [`Self::redeploy`]'s
    /// decision input and nothing else asks it — the panel compares it against
    /// the running image to decide whether a redeploy would move anything. The
    /// status readers cannot answer it: they see the pod, not the deployment
    /// config, so every one of them reports `latest_sandbox_image: None`.
    async fn expected_image(&self, task_id: &str) -> Result<String, SandboxError>;
}

/// A lifecycle operation failure, band-clean and reduced to what the task API
/// surfaces. The sandbox-band implementation pre-classifies each variant from
/// the rich `LifecycleError` so the tasks band names no `sandbox` error type.
#[derive(Debug, thiserror::Error)]
pub enum SandboxOpError {
    /// The Task has no row — maps to a 404.
    #[error("task not found: {0}")]
    NotFound(String),
    /// The operation was rejected for the Task's current state (the `Closed`
    /// guard) — a client state error, maps to a 400.
    #[error("{0}")]
    NotAllowed(String),
    /// Another operation holds the per-Task lock — maps to a 409.
    #[error("{0}")]
    InProgress(String),
    /// Any other lifecycle failure — maps to a 500 with the rendered chain.
    #[error("{0}")]
    Internal(String),
}

/// The durable sandbox-lifecycle contract the tasks band drives.
///
/// The task closer, reaper, HTTP handlers, and service drive a Task's sandbox —
/// start it, stop it, remove it, read its desired state — but they are a band
/// below the `Lifecycle` orchestrator that owns those transitions. This port
/// inverts that edge: the consumers hold `Arc<dyn SandboxLifecycle>` and name no
/// `sandbox` type, and `Lifecycle` implements it at the composition root.
///
/// It is deliberately **not** [`SandboxStarter`]/[`SandboxStopper`]
/// /[`SandboxIntentReader`]: those flatten a lifecycle failure through
/// [`SandboxError`], which has no `Conflict` variant, so lock contention
/// (`OperationInProgress`) that the task API answers **409** would arrive as an
/// internal **500**. The tasks band needs that 409, so this contract's error
/// preserves the three client-visible outcomes. The
/// `LifecycleError -> SandboxOpError` mapping lives in the sandbox-band
/// implementation; `moatless_tasks::task::error::TaskError` maps
/// `SandboxOpError` the rest of the way (404 / 400 / 409 / 500).
///
/// Sized to the tasks-band cross-boundary surface: the concrete `Lifecycle` also
/// serves `restart`/`redeploy`/`expected_sandbox_image`, but those have no
/// tasks-band caller (only the sandbox HTTP handlers, on the legal side of the
/// boundary), so they stay off the port.
#[async_trait]
pub trait SandboxLifecycle: Send + Sync {
    /// Provision and start the Task's sandbox.
    async fn start(&self, task_id: &str) -> Result<(), SandboxOpError>;

    /// Stop the Task's sandbox, recording why.
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxOpError>;

    /// Stop the agent running inside the Task's sandbox, leaving the sandbox
    /// up. Not a lifecycle transition: nothing is recorded and the desired
    /// state is untouched, so the next turn runs in the same sandbox.
    ///
    /// Best-effort, like the graceful stop [`Self::stop`] issues before it
    /// tears compute down: a sandbox that cannot be reached is one whose agent
    /// is not running anyway. The terminal `agent.stopped` the caller publishes
    /// is what closes the turn.
    async fn stop_agent(&self, task_id: &str, reason: SandboxStopReason);

    /// Remove the Task's sandbox, applying the archive-retention policy. The
    /// caller deletes the Task row only on success.
    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
    ) -> Result<(), SandboxOpError>;

    /// Reclaim the storage of a Task whose sandbox has been stopped long
    /// enough, leaving the durable archive as the only copy. The Task stays
    /// Open and its sandbox stays `Stopped` — a later start restores from the
    /// archive — so this is a cost transition, not a state one, and the reaper
    /// is its only caller.
    ///
    /// Idempotent: a Task whose storage is already reclaimed is not an error.
    async fn archive_storage(&self, task_id: &str) -> Result<(), SandboxOpError>;

    /// Which of `task_ids` have a stopped sandbox that still owns storage, and
    /// when each stopped. The candidate set for [`Self::archive_storage`]: the
    /// caller applies its own per-Task clock on top, because the threshold is a
    /// Task property and the storage is a sandbox one.
    async fn stopped_with_storage(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, SandboxOpError>;

    /// The persisted desired state for the Task's sandbox (absence reads as
    /// converged `Stopped`).
    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxOpError>;
}
