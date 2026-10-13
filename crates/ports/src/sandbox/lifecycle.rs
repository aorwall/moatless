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

/// The lifecycle surface the T3 sandbox panel drives: three transitions plus
/// the read that decides whether to offer one.
///
/// Deliberately not [`SandboxLifecycle`]: these flatten through
/// [`SandboxError`], like [`SandboxStarter`]/[`SandboxStopper`], because the T3
/// panel renders one refusal for every failure.
#[async_trait]
pub trait SandboxOperator: Send + Sync {
    /// Replace the Task's sandbox pod, keeping its deployment and its
    /// workspace.
    async fn restart(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Rebuild the Task's sandbox deployment from current repository config,
    /// keeping its workspace volume. What picks up a new image.
    async fn redeploy(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Remove the Task's sandbox and delete its archives, leaving the Task.
    ///
    /// Retention is fixed to [`ArchiveRetention::DeleteArchives`], the only
    /// option the REST cleanup route applies.
    async fn cleanup(&self, task_id: &str) -> Result<(), SandboxError>;

    /// The image a fresh deploy would install: the per-repo `docker_image`
    /// override, else the configured default.
    ///
    /// [`Self::redeploy`]'s decision input — the panel compares it against the
    /// running image. Status readers can't answer this: they see the pod, not
    /// the deployment config.
    async fn expected_image(&self, task_id: &str) -> Result<String, SandboxError>;
}

/// A lifecycle operation failure, band-clean and reduced to what the task API
/// surfaces.
///
/// Pre-classified from the rich `LifecycleError` so the tasks band names no
/// `sandbox` error type.
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
/// Deliberately not [`SandboxStarter`]/[`SandboxStopper`]/
/// [`SandboxIntentReader`]: those flatten through [`SandboxError`] (no
/// `Conflict`), so a 409 would arrive as a 500. Excludes HTTP-only
/// `restart`/`redeploy`/`expected_sandbox_image`.
#[async_trait]
pub trait SandboxLifecycle: Send + Sync {
    /// Provision and start the Task's sandbox.
    async fn start(&self, task_id: &str) -> Result<(), SandboxOpError>;

    /// Stop the Task's sandbox, recording why.
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxOpError>;

    /// Stop the agent inside the Task's sandbox, leaving it up — not a
    /// lifecycle transition, so the next turn runs in the same sandbox.
    ///
    /// Best-effort, like [`Self::stop`]: an unreachable sandbox has no agent
    /// running anyway. The caller's terminal `agent.stopped` closes the turn.
    async fn stop_agent(&self, task_id: &str, reason: SandboxStopReason);

    /// Remove the Task's sandbox, applying the archive-retention policy. The
    /// caller deletes the Task row only on success.
    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
    ) -> Result<(), SandboxOpError>;

    /// Reclaim the storage of a Task whose sandbox has been stopped long
    /// enough, leaving the durable archive as the only copy.
    ///
    /// The Task stays Open and `Stopped` — a later start restores from the
    /// archive — so this is a cost transition, not a state one. Idempotent; the
    /// reaper is its only caller.
    async fn archive_storage(&self, task_id: &str) -> Result<(), SandboxOpError>;

    /// Which of `task_ids` have a stopped sandbox that still owns storage, and
    /// when each stopped.
    ///
    /// The candidate set for [`Self::archive_storage`]; the caller applies its
    /// own per-Task clock since the threshold is a Task property, not a sandbox
    /// one.
    async fn stopped_with_storage(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, SandboxOpError>;

    /// The persisted desired state for the Task's sandbox (absence reads as
    /// converged `Stopped`).
    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxOpError>;
}
