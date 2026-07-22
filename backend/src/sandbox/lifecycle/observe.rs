//! Production [`FleetObserver`] adapters over the Kubernetes and Docker runtimes.
//!
//! `observe_fleet` is one label-selector LIST per tick, mapped to the coarse
//! [`SandboxObservedStatus`] the reconciler joins against intent. `observe` is
//! the authoritative per-Task confirm, delegated to the provider's live-status
//! projection so a divergence is verified before any operation is driven.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::SandboxProviderType;
use crate::sandbox::docker::client::DockerClient;
use crate::sandbox::k8s::client::K8sClient;
use crate::sandbox::provider::SandboxProvider;
use crate::task::models::{SandboxDesiredState, TaskStatus};

use super::convergence::SandboxObservedStatus;
use super::reconciler::{FleetObserver, NotifyPort, ObserveError};

/// Confirm a Task's status authoritatively via the provider's live-status read.
async fn confirm(
    provider: &Arc<dyn SandboxProvider>,
    task_id: &str,
    task_status: TaskStatus,
    desired_state: SandboxDesiredState,
) -> Result<SandboxObservedStatus, ObserveError> {
    let live = provider
        .live_status(task_id, task_status, desired_state)
        .await
        .map_err(|e| ObserveError::with_source("confirm live status", e))?;
    Ok(SandboxObservedStatus::from_display(live.sandbox_status))
}

/// Kubernetes fleet observer: one Deployment LIST → coarse status per Task.
pub struct K8sFleetObserver {
    k8s: K8sClient,
    provider: Arc<dyn SandboxProvider>,
}

impl K8sFleetObserver {
    pub fn new(k8s: K8sClient, provider: Arc<dyn SandboxProvider>) -> Self {
        Self { k8s, provider }
    }
}

#[async_trait]
impl FleetObserver for K8sFleetObserver {
    async fn observe_fleet(&self) -> Result<HashMap<String, SandboxObservedStatus>, ObserveError> {
        let states = crate::sandbox::k8s::deployment::list_sandbox_deployment_states(&self.k8s)
            .await
            .map_err(|e| ObserveError::with_source("list sandbox deployments", e))?;
        Ok(states
            .into_iter()
            .map(|(task_id, desired, ready)| {
                // A Deployment scaled to zero is Stopped; otherwise ready replicas
                // decide Ready vs. still-Starting.
                let status = if desired == 0 {
                    SandboxObservedStatus::Stopped
                } else if ready > 0 {
                    SandboxObservedStatus::Ready
                } else {
                    SandboxObservedStatus::Starting
                };
                (task_id, status)
            })
            .collect())
    }

    async fn observe(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<SandboxObservedStatus, ObserveError> {
        confirm(&self.provider, task_id, task_status, desired_state).await
    }
}

/// Docker fleet observer: one container LIST (label `moatless.service=sandbox`)
/// → coarse status per Task.
pub struct DockerFleetObserver {
    docker: DockerClient,
    provider: Arc<dyn SandboxProvider>,
}

impl DockerFleetObserver {
    pub fn new(docker: DockerClient, provider: Arc<dyn SandboxProvider>) -> Self {
        Self { docker, provider }
    }
}

#[async_trait]
impl FleetObserver for DockerFleetObserver {
    async fn observe_fleet(&self) -> Result<HashMap<String, SandboxObservedStatus>, ObserveError> {
        use bollard::models::ContainerSummaryStateEnum as State;
        use bollard::query_parameters::ListContainersOptionsBuilder;

        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec!["moatless.service=sandbox".to_string()],
        );
        let containers = self
            .docker
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::new()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| ObserveError::with_source("list sandbox containers", e))?;

        let mut out = HashMap::new();
        for c in containers {
            let Some(task_id) = c
                .labels
                .as_ref()
                .and_then(|l| l.get("moatless.io/task-id"))
                .cloned()
            else {
                continue;
            };
            let status = match c.state {
                Some(State::RUNNING) => SandboxObservedStatus::Ready,
                Some(State::CREATED | State::RESTARTING | State::STOPPING) => {
                    SandboxObservedStatus::Starting
                }
                Some(State::REMOVING) => SandboxObservedStatus::Terminating,
                _ => SandboxObservedStatus::Stopped,
            };
            out.insert(task_id, status);
        }
        Ok(out)
    }

    async fn observe(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<SandboxObservedStatus, ObserveError> {
        confirm(&self.provider, task_id, task_status, desired_state).await
    }
}

/// gRPC notifier: pings a Ready sandbox to fetch its pending messages, over the
/// provider's service/container DNS.
pub struct GrpcNotify {
    provider: SandboxProviderType,
    namespace: String,
    grpc_port: u16,
}

impl GrpcNotify {
    pub fn new(provider: SandboxProviderType, namespace: String, grpc_port: u16) -> Self {
        Self {
            provider,
            namespace,
            grpc_port,
        }
    }
}

#[async_trait]
impl NotifyPort for GrpcNotify {
    async fn notify(&self, task_id: &str) {
        crate::sandbox::notify::notify_task_over_grpc(
            self.provider,
            &self.namespace,
            self.grpc_port,
            task_id,
        )
        .await;
    }
}
