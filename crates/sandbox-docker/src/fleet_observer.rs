//! Docker [`FleetObserver`] adapter: one container LIST per tick mapped to the
//! coarse [`SandboxObservedStatus`] the reconciler joins against intent, plus the
//! authoritative per-Task confirm delegated to the observer's live-status read.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use moatless_sandbox_control::lifecycle::convergence::SandboxObservedStatus;
use moatless_sandbox_control::lifecycle::observe::confirm;
use moatless_sandbox_control::lifecycle::reconciler::{FleetObserver, ObserveError, RuntimeObservation};
use moatless_sandbox_control::lifecycle::runtime::{generation_from_label, OPERATION_GENERATION_LABEL};
use moatless_sandbox_control::status_reader::SandboxStatusReader;
use moatless_tasks::task::models::{SandboxDesiredState, TaskStatus};

use super::client::DockerClient;
use super::container_builder::{SANDBOX_SERVICE, SERVICE_LABEL};
use super::naming::generate_container_name;

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
                config
                    .labels
                    .and_then(|labels| generation_from_label(labels.get(OPERATION_GENERATION_LABEL)))
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
