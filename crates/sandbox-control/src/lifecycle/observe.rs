//! Lifecycle-owned observation helpers shared by the provider adapters: the
//! authoritative live-status [`confirm`] the [`FleetObserver`] implementations
//! delegate to, and the gRPC [`NotifyPort`].
//!
//! [`FleetObserver`]: super::reconciler::FleetObserver

use std::sync::Arc;

use async_trait::async_trait;

use crate::status_reader::SandboxStatusReader;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskStatus;
use crate::client::{SandboxConnector, classify_grpc_error};

use super::convergence::SandboxObservedStatus;
use super::orchestrator::SandboxMember;
use super::reconciler::{MemberRegistryError, MemberRegistryPort, NotifyPort, ObserveError};

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

/// gRPC member registrar: rebuilds in-pod Task contexts from durable DB
/// membership after pod restarts.
pub struct GrpcMemberRegistry {
    connector: Arc<dyn SandboxConnector>,
}

impl GrpcMemberRegistry {
    pub fn new(connector: Arc<dyn SandboxConnector>) -> Self {
        Self { connector }
    }
}

#[async_trait]
impl MemberRegistryPort for GrpcMemberRegistry {
    async fn ensure_registered(&self, member: &SandboxMember) -> Result<(), MemberRegistryError> {
        let client = self
            .connector
            .sandbox_client(&member.task_id)
            .await
            .map_err(|e| MemberRegistryError::with_source("connect sandbox for JoinTask", e))?;
        client
            .join_task(&member.task_id, Some(member.checkout_mode.as_str()))
            .await
            .map_err(|e| {
                MemberRegistryError::with_source(
                    "JoinTask failed",
                    classify_grpc_error(e, "JoinTask"),
                )
            })
    }
}
