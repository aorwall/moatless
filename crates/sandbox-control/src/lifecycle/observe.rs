//! Lifecycle-owned observation helpers shared by the provider adapters: the
//! authoritative live-status [`confirm`] the [`FleetObserver`] implementations
//! delegate to, and the gRPC [`NotifyPort`].
//!
//! [`FleetObserver`]: super::reconciler::FleetObserver

use std::sync::Arc;

use async_trait::async_trait;

use crate::status_reader::SandboxStatusReader;
use moatless_tasks::task::models::{SandboxDesiredState, TaskStatus};
use moatless_backend_core::sandbox::SandboxConnector;

use super::convergence::SandboxObservedStatus;
use super::reconciler::{NotifyPort, ObserveError};

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

/// gRPC notifier: pings a Ready sandbox to fetch its pending messages, over the
/// connector's service/container addressing.
pub struct GrpcNotify {
    connector: Arc<dyn SandboxConnector>,
}

impl GrpcNotify {
    pub fn new(connector: Arc<dyn SandboxConnector>) -> Self {
        Self { connector }
    }
}

#[async_trait]
impl NotifyPort for GrpcNotify {
    async fn notify(&self, task_id: &str) {
        crate::notify::notify_task_over_grpc(self.connector.as_ref(), task_id).await;
    }
}
