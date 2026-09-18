//! [`FleetObserver`] for Kubernetes: one Deployment LIST and one Service LIST
//! per tick, whatever the fleet's size.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use moatless_sandbox_runtime::convergence::SandboxObservedStatus;
use moatless_sandbox_runtime::fleet::{FleetObserver, ObserveError, RuntimeObservation};
use moatless_sandbox_runtime::runtime::{OPERATION_GENERATION_LABEL, generation_from_label};
use moatless_sandbox_runtime::status_reader::SandboxStatusReader;
use moatless_sandbox_runtime::status_reader::confirm;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskStatus;

use super::client::K8sClient;
use super::deployment::SandboxDeploymentState;

/// The coarse status a Deployment's replica counts imply, given whether its
/// sandbox Service is present.
///
/// `serviced` is `None` when the Service LIST failed; an unreadable list is not
/// evidence the Service is gone, so it leaves each Deployment's own readiness
/// standing rather than restarting the fleet on one failed call.
fn observed_status(
    state: &SandboxDeploymentState,
    serviced: Option<&HashSet<String>>,
) -> SandboxObservedStatus {
    if state.desired_replicas == 0 {
        return SandboxObservedStatus::Stopped;
    }
    if state.ready_replicas == 0 {
        return SandboxObservedStatus::Starting;
    }
    // A ready pod behind a missing Service is unreachable: every gRPC and file
    // call the backend makes routes through that Service, so the pod's own
    // readiness proves nothing. `Ready` would converge the reconciler and strand
    // the sandbox; `Error` drives `Start`, whose network heal recreates it.
    if serviced.is_some_and(|ids| !ids.contains(&state.task_id)) {
        return SandboxObservedStatus::Error;
    }
    SandboxObservedStatus::Ready
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
        let states = super::deployment::list_sandbox_deployment_states(&self.k8s)
            .await
            .map_err(|e| ObserveError::with_source("list sandbox deployments", e))?;
        let serviced = match super::service::list_sandbox_service_task_ids(&self.k8s).await {
            Ok(ids) => Some(ids),
            Err(e) => {
                warn!(error = %e, "Failed to list sandbox Services; fleet readiness ignores Service presence this tick");
                None
            }
        };
        Ok(states
            .into_iter()
            .map(|state| {
                let status = observed_status(&state, serviced.as_ref());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn state(desired: i32, ready: i32) -> SandboxDeploymentState {
        SandboxDeploymentState {
            task_id: "t1".to_string(),
            desired_replicas: desired,
            ready_replicas: ready,
            operation_generation: None,
        }
    }

    fn serviced(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn a_ready_deployment_with_its_service_is_ready() {
        assert_eq!(
            observed_status(&state(1, 1), Some(&serviced(&["t1"]))),
            SandboxObservedStatus::Ready
        );
    }

    /// The whole point of the Service LIST: `Ready` here would converge the
    /// reconciler onto a sandbox nothing can reach, so it never re-drives
    /// `ensure_running` and the missing Service is never recreated.
    #[test]
    fn a_ready_deployment_without_its_service_is_an_error() {
        assert_eq!(
            observed_status(&state(1, 1), Some(&serviced(&["other"]))),
            SandboxObservedStatus::Error
        );
    }

    #[test]
    fn an_unreadable_service_list_leaves_readiness_standing() {
        assert_eq!(
            observed_status(&state(1, 1), None),
            SandboxObservedStatus::Ready
        );
    }

    /// A scaled-to-zero sandbox keeps its Service, and a stopped one is
    /// converged whether or not the Service outlived it.
    #[test]
    fn a_scaled_down_deployment_is_stopped_regardless_of_its_service() {
        assert_eq!(
            observed_status(&state(0, 0), Some(&serviced(&[]))),
            SandboxObservedStatus::Stopped
        );
    }

    /// A pod still coming up has no Service to miss yet — provisioning creates
    /// the Deployment first, so `Error` here would fire on every cold start.
    #[test]
    fn a_deployment_with_no_ready_replicas_is_starting() {
        assert_eq!(
            observed_status(&state(1, 0), Some(&serviced(&[]))),
            SandboxObservedStatus::Starting
        );
    }
}
