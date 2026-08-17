//! Kubernetes [`FleetObserver`] adapter: one Deployment LIST per tick mapped to
//! the coarse [`SandboxObservedStatus`] the reconciler joins against intent, plus
//! the authoritative per-Task confirm delegated to the observer's live-status read.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use moatless_backend_core::models::{SandboxDesiredState, TaskStatus};
use moatless_sandbox_control::lifecycle::convergence::SandboxObservedStatus;
use moatless_sandbox_control::lifecycle::observe::confirm;
use moatless_sandbox_control::lifecycle::reconciler::{
    FleetObserver, ObserveError, RuntimeObservation,
};
use moatless_sandbox_control::lifecycle::runtime::{
    OPERATION_GENERATION_LABEL, generation_from_label,
};
use moatless_sandbox_control::status_reader::SandboxStatusReader;

use super::client::K8sClient;

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
        let states = super::deployment::list_sandbox_deployment_states(&self.k8s)
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
        let generation = super::deployment::find_deployment(&self.k8s, task_id)
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
