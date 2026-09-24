//! The backend's full-fidelity sandbox read, for the HTTP status and log
//! routes.
//!
//! `handlers` serves the thirteen-field `SandboxStatusFullResponse`, so it reads
//! through this trait; the narrower `moatless_ports::sandbox::SandboxObserver`
//! is projected from this read in `t3code::projection`. Errors are the sandbox
//! module's `SandboxError`, which carries an axum `IntoResponse`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::convergence::SandboxObservedStatus;
use crate::fleet::ObserveError;
use futures::stream::BoxStream;

use super::access::LogOptions;
use super::error::SandboxError;
use super::live_status::LiveStatus;
use super::schemas::SandboxStatusFullResponse;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskStatus;

/// Live runtime observation used by the sandbox status and diagnostics routes.
#[async_trait]
pub trait SandboxStatusReader: Send + Sync {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError>;

    async fn live_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<LiveStatus, SandboxError>;

    /// One value per log line, and nothing about how a line is delivered.
    ///
    /// The SSE route frames each line at its own boundary; an `axum` SSE
    /// `Event` is write-only and could not be re-emitted over a second
    /// transport.
    async fn sandbox_logs(
        &self,
        task_id: &str,
        options: LogOptions,
    ) -> Result<BoxStream<'static, Result<String, SandboxError>>, SandboxError>;
}

/// Confirm a Task's status authoritatively via the observer's live-status read.
pub async fn confirm(
    observer: &Arc<dyn SandboxStatusReader>,
    task_id: &str,
    task_status: TaskStatus,
    desired_state: SandboxDesiredState,
) -> Result<SandboxObservedStatus, ObserveError> {
    let live = observer
        .live_status(task_id, task_status, desired_state)
        .await
        .map_err(|e| ObserveError::with_source("confirm live status", e))?;
    Ok(SandboxObservedStatus::from_display(live.sandbox_status))
}
