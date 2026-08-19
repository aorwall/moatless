//! The tasks-band view of the sandbox lifecycle.
//!
//! The task closer, reaper, HTTP handlers, and service drive a Task's sandbox —
//! start it, stop it, remove it, read its desired state — but they are a band
//! below the `Lifecycle` orchestrator that owns those transitions. This port
//! inverts that edge: the consumers hold `Arc<dyn SandboxLifecycle>` and name no
//! `sandbox` type, and `Lifecycle` implements it at the composition root.
//!
//! It is deliberately **not** the T3 contract's `SandboxStarter`/`SandboxStopper`
//! /`SandboxIntentReader`: those flatten a lifecycle failure through
//! `core_sandbox::SandboxError`, which has no `Conflict` variant, so lock
//! contention (`OperationInProgress`) that the task API answers **409** would
//! arrive as an internal **500**. The tasks band needs that 409, so it owns its
//! own lifecycle contract whose error preserves the three client-visible
//! outcomes. The `LifecycleError -> SandboxOpError` mapping lives in the
//! sandbox-band implementation; [`TaskError`](crate::task::error::TaskError) maps
//! `SandboxOpError` the rest of the way (404 / 400 / 409 / 500).

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::task::models::{SandboxDesiredState, SandboxStopReason};
use moatless_types::sandbox::ArchiveRetention;

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

/// The durable sandbox-lifecycle operations the tasks band drives. Sized to the
/// tasks-band cross-boundary surface: the concrete `Lifecycle` also serves
/// `restart`/`redeploy`/`expected_sandbox_image`, but those have no tasks-band
/// caller (only the sandbox HTTP handlers, on the legal side of the boundary),
/// so they stay off the port.
#[async_trait]
pub trait SandboxLifecycle: Send + Sync {
    /// Provision and start the Task's sandbox.
    async fn start(&self, task_id: &str) -> Result<(), SandboxOpError>;

    /// Stop the Task's sandbox, recording why.
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxOpError>;

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
