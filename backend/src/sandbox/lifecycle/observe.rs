//! Production [`FleetObserver`] adapters over the Kubernetes and Docker runtimes.
//!
//! `observe_fleet` is one label-selector LIST per tick, mapped to the coarse
//! [`SandboxObservedStatus`] the reconciler joins against intent. `observe` is
//! the authoritative per-Task confirm, delegated to the observer's live-status
//! projection so a divergence is verified before any operation is driven.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::sandbox::status_reader::SandboxStatusReader;
use moatless_backend_core::sandbox::SandboxConnector;
use crate::sandbox::docker::client::DockerClient;
use crate::sandbox::docker::container_builder::{SANDBOX_SERVICE, SERVICE_LABEL};
use crate::sandbox::docker::naming::generate_container_name;
use crate::sandbox::k8s::client::K8sClient;
use crate::task::models::{SandboxDesiredState, TaskStatus};

use super::convergence::SandboxObservedStatus;
use super::reconciler::{FleetObserver, NotifyPort, ObserveError, RuntimeObservation};
use super::runtime::{generation_from_label, OPERATION_GENERATION_LABEL};

/// Confirm a Task's status authoritatively via the observer's live-status read.
async fn confirm(
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

/// Kubernetes fleet observer: one Deployment LIST → coarse status per Task.
pub struct K8sFleetObserver {
    k8s: K8sClient,
    observer: Arc<dyn SandboxStatusReader>,
}

impl K8sFleetObserver {
    pub fn new(k8s: K8sClient, observer: Arc<dyn SandboxStatusReader>) -> Self {
        Self { k8s, observer }
    }
}

#[async_trait]
impl FleetObserver for K8sFleetObserver {
    async fn observe_fleet(&self) -> Result<HashMap<String, RuntimeObservation>, ObserveError> {
        let states = crate::sandbox::k8s::deployment::list_sandbox_deployment_states(&self.k8s)
            .await
            .map_err(|e| ObserveError::with_source("list sandbox deployments", e))?;
        Ok(states
            .into_iter()
            .map(|state| {
                // A Deployment scaled to zero is Stopped; otherwise ready replicas
                // decide Ready vs. still-Starting.
                let status = if state.desired_replicas == 0 {
                    SandboxObservedStatus::Stopped
                } else if state.ready_replicas > 0 {
                    SandboxObservedStatus::Ready
                } else {
                    SandboxObservedStatus::Starting
                };
                (
                    state.task_id,
                    RuntimeObservation::new(status, state.operation_generation),
                )
            })
            .collect())
    }

    async fn observe(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<RuntimeObservation, ObserveError> {
        let status = confirm(&self.observer, task_id, task_status, desired_state).await?;
        let generation = crate::sandbox::k8s::deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|e| ObserveError::with_source("inspect sandbox deployment generation", e))?
            .and_then(|deployment| {
                deployment
                    .spec
                    .and_then(|spec| spec.template.metadata)
                    .and_then(|metadata| metadata.labels)
                    .and_then(|labels| {
                        generation_from_label(labels.get(OPERATION_GENERATION_LABEL))
                    })
            });
        Ok(RuntimeObservation::new(status, generation))
    }
}

/// Docker fleet observer: one container LIST (label `moatless.service=sandbox`)
/// → coarse status per Task.
pub struct DockerFleetObserver {
    docker: DockerClient,
    observer: Arc<dyn SandboxStatusReader>,
}

impl DockerFleetObserver {
    pub fn new(docker: DockerClient, observer: Arc<dyn SandboxStatusReader>) -> Self {
        Self { docker, observer }
    }
}

#[async_trait]
impl FleetObserver for DockerFleetObserver {
    async fn observe_fleet(&self) -> Result<HashMap<String, RuntimeObservation>, ObserveError> {
        use bollard::models::ContainerSummaryStateEnum as State;
        use bollard::query_parameters::ListContainersOptionsBuilder;

        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![format!("{SERVICE_LABEL}={SANDBOX_SERVICE}")],
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
            let generation = c
                .labels
                .as_ref()
                .and_then(|labels| generation_from_label(labels.get(OPERATION_GENERATION_LABEL)));
            out.insert(task_id, RuntimeObservation::new(status, generation));
        }
        Ok(out)
    }

    async fn observe(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<RuntimeObservation, ObserveError> {
        let status = confirm(&self.observer, task_id, task_status, desired_state).await?;
        let generation = match self
            .docker
            .docker
            .inspect_container(&generate_container_name(task_id), None)
            .await
        {
            Ok(container) => container.config.and_then(|config| {
                config.labels.and_then(|labels| {
                    generation_from_label(labels.get(OPERATION_GENERATION_LABEL))
                })
            }),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => None,
            Err(e) => {
                return Err(ObserveError::with_source(
                    "inspect sandbox container generation",
                    e,
                ));
            }
        };
        Ok(RuntimeObservation::new(status, generation))
    }
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
        crate::sandbox::notify::notify_task_over_grpc(self.connector.as_ref(), task_id).await;
    }
}
