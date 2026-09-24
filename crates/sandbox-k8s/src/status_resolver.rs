//! Read-only domain service resolving sandbox status from the K8s API.

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use tracing::warn;

use super::client::K8sClient;
use super::deployment;
use super::pod_reader;
use super::pod_status::{
    check_sandbox_permanent_failure, is_sandbox_container_ready, pick_best_pod,
};
use super::service;
use moatless_sandbox_control::status::StatusResolve;
use moatless_sandbox_runtime::client::{AgentStatusSnapshot, SandboxConnector, fetch_agent_status};
use moatless_sandbox_runtime::container_status::{ContainerState, PodPhase};
use moatless_sandbox_runtime::live_status::{LiveStatus, derive_live_status};
use moatless_sandbox_runtime::schemas::*;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::sandbox::SandboxStatus;
use moatless_types::task::TaskStatus;

/// Read-only domain service for resolving sandbox status from the K8s API.
///
/// Pure infrastructure, with no DB dependencies.
pub struct StatusResolver {
    k8s: K8sClient,
}

impl StatusResolver {
    pub fn new(k8s: K8sClient) -> Self {
        Self { k8s }
    }
}

#[async_trait]
impl StatusResolve for StatusResolver {
    async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> SandboxStatusFullResponse {
        let (phase, resolver_error, current_sandbox_image) = self
            .resolve_pod_phase_and_error(sandbox_id, desired_state)
            .await;
        let sandbox_status = phase.sandbox_status();
        let is_running = phase == PodPhase::Running;

        if is_running {
            // Query actual agent status from the sandbox pod via gRPC
            let AgentStatusSnapshot {
                status: agent_status,
                scheduled_jobs,
                commands,
            } = fetch_agent_status(task_id, connector).await;

            let live_status = derive_live_status(
                task_status,
                desired_state,
                sandbox_status,
                resolver_error,
                agent_status,
            );

            // Fetch all container statuses for diagnostics
            let container_diagnostics = self.fetch_container_diagnostics(sandbox_id).await;

            return SandboxStatusFullResponse {
                sandbox_status: live_status.sandbox_status,
                sandbox_error: live_status.sandbox_error,
                agent_status: live_status.agent_status,
                resources: None,
                container_diagnostics,
                runtime_events: None,
                scheduled_jobs,
                commands,
                sandbox_desired_state: None,
                current_sandbox_image,
                latest_sandbox_image: None,
                image_up_to_date: None,
            };
        }

        // Fetch diagnostics for non-running pods with issues
        let fetch_diagnostics = matches!(
            phase,
            PodPhase::Pending | PodPhase::Failed | PodPhase::Unknown
        );
        let container_diagnostics = if fetch_diagnostics {
            self.fetch_container_diagnostics(sandbox_id).await
        } else {
            None
        };

        // Fetch runtime events for error/initializing states
        let runtime_events = if fetch_diagnostics {
            let events = pod_reader::fetch_pod_runtime_events(&self.k8s, sandbox_id).await;
            if events.is_empty() {
                None
            } else {
                Some(events)
            }
        } else {
            None
        };

        let live_status = derive_live_status(
            task_status,
            desired_state,
            sandbox_status,
            resolver_error,
            None,
        );

        SandboxStatusFullResponse {
            sandbox_status: live_status.sandbox_status,
            sandbox_error: live_status.sandbox_error,
            agent_status: live_status.agent_status,
            resources: None,
            container_diagnostics,
            runtime_events,
            scheduled_jobs: Vec::new(),
            commands: Vec::new(),
            sandbox_desired_state: None,
            current_sandbox_image,
            latest_sandbox_image: None,
            image_up_to_date: None,
        }
    }

    async fn resolve_live_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> LiveStatus {
        let (phase, resolver_error, _) = self
            .resolve_pod_phase_and_error(sandbox_id, desired_state)
            .await;
        let sandbox_status = phase.sandbox_status();
        // One gRPC call answers both halves. Taking only `status` here is what
        // left a reader unable to see that `moat cmd` work was still running
        // behind an idle agent.
        let (agent_status, commands) = if sandbox_status == SandboxStatus::Ready {
            let snapshot = fetch_agent_status(task_id, connector).await;
            let commands = snapshot
                .commands
                .iter()
                .map(ManagedCommand::summary)
                .collect();
            (snapshot.status, commands)
        } else {
            (None, Vec::new())
        };

        derive_live_status(
            task_status,
            desired_state,
            sandbox_status,
            resolver_error,
            agent_status,
        )
        .with_commands(commands)
    }
}

impl StatusResolver {
    // =========================================================================
    // Private helpers
    // =========================================================================

    /// Fetch container diagnostics for all containers in the pod,
    /// including resource usage from the K8s Metrics API when available.
    async fn fetch_container_diagnostics(
        &self,
        task_id: &str,
    ) -> Option<Vec<ContainerDiagnostics>> {
        let all_statuses = pod_reader::fetch_all_container_statuses(&self.k8s, task_id).await;
        if all_statuses.is_empty() {
            return None;
        }

        // Fetch resource usage (best-effort — returns empty map if metrics-server unavailable)
        let resource_usage = pod_reader::fetch_container_resource_usage(&self.k8s, task_id).await;

        Some(
            all_statuses
                .into_iter()
                .map(|cs| {
                    let usage = resource_usage.get(&cs.name);
                    ContainerDiagnostics {
                        name: cs.name,
                        state: match cs.state {
                            ContainerState::Running => "running".to_string(),
                            ContainerState::Waiting => "waiting".to_string(),
                            ContainerState::Terminated => "terminated".to_string(),
                            ContainerState::Unknown => "unknown".to_string(),
                        },
                        restart_count: cs.restart_count,
                        exit_code: cs.exit_code,
                        terminated_reason: cs.terminated_reason,
                        last_exit_code: cs.last_exit_code,
                        last_terminated_reason: cs.last_terminated_reason,
                        waiting_reason: cs.waiting_reason,
                        cpu_millicores: usage.map(|u| u.cpu_millicores),
                        memory_mb: usage.map(|u| u.memory_bytes / (1024 * 1024)),
                    }
                })
                .collect(),
        )
    }

    /// Whether a ready pod is actually reachable. See the call site.
    ///
    /// The backend reaches a sandbox only through its Service, so a ready pod
    /// with no Service must not read as `Running`; `Failed` re-drives
    /// `ensure_running`, whose network heal recreates it.
    async fn running_or_unreachable(&self, task_id: &str) -> (PodPhase, Option<String>) {
        match service::sandbox_service_exists(&self.k8s, task_id).await {
            Ok(true) => (PodPhase::Running, None),
            Ok(false) => (
                PodPhase::Failed,
                Some(
                    "The sandbox pod is running but its Service is missing, so the backend \
                     cannot reach it. Recovery recreates the Service."
                        .to_string(),
                ),
            ),
            // An unreadable Service is not evidence of absence; the running pod
            // is the answer that predates this branch.
            Err(e) => {
                warn!(task_id, error = %e, "Could not read the sandbox Service, reporting Running");
                (PodPhase::Running, None)
            }
        }
    }

    /// What no Deployment means for this Task. See the call site.
    async fn absent_phase(&self, task_id: &str, desired_state: SandboxDesiredState) -> PodPhase {
        if desired_state != SandboxDesiredState::Stopped {
            return PodPhase::NotFound;
        }
        match service::sandbox_service_exists(&self.k8s, task_id).await {
            Ok(true) => PodPhase::Stopped,
            Ok(false) => PodPhase::NotFound,
            Err(e) => {
                // An unreadable Service is not evidence either way; the
                // never-created answer is the one that predates this branch.
                warn!(task_id, error = %e, "Could not read the sandbox Service, reporting NotFound");
                PodPhase::NotFound
            }
        }
    }

    /// Resolve the effective pod phase, any permanent error, and the
    /// provisioned sandbox image, checking Deployment replicas first.
    ///
    /// Replicas=0 returns `(Stopped, None, image)`; otherwise falls back to pod
    /// status and permanent-failure probes. `desired_state` is read only on the
    /// no-Deployment path, to distinguish never-existed from reclaimed.
    async fn resolve_pod_phase_and_error(
        &self,
        task_id: &str,
        desired_state: SandboxDesiredState,
    ) -> (PodPhase, Option<String>, Option<String>) {
        // Check Deployment replicas first
        let current_image =
            if let Ok(Some(dep)) = deployment::find_deployment(&self.k8s, task_id).await {
                let image = deployment::deployment_sandbox_image(&dep);
                if let Ok((desired, _ready)) =
                    deployment::get_deployment_replicas(&self.k8s, task_id).await
                {
                    if desired == 0 {
                        return (PodPhase::Stopped, None, image);
                    }
                }
                image
            } else {
                // No Deployment. That is "never created" for a Task that never
                // ran, but it is also what a reclaimed sandbox looks like — the
                // cold transition deletes the Deployment and the volume and
                // deliberately keeps the Service, so a surviving Service is the
                // mark of a sandbox whose compute was reclaimed. Only asked when
                // intent is Stopped: a Task mid-remove has its Deployment gone
                // and its Service still draining, and that is `NotCreated` (which
                // the Closed projection renders as Removed), not Stopped.
                return (self.absent_phase(task_id, desired_state).await, None, None);
            };

        // Deployment exists with replicas > 0, check actual pod status
        let pods_api: Api<Pod> = Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

        match pods_api.list(&lp).await {
            Ok(list) => {
                if let Some(pod) = pick_best_pod(&list.items) {
                    let is_terminating = pod.metadata.deletion_timestamp.is_some();
                    let raw_phase = pod
                        .status
                        .as_ref()
                        .and_then(|s| s.phase.as_deref())
                        .unwrap_or("Unknown");

                    // Check for permanent sandbox failures (CrashLoopBackOff, ImagePullBackOff, etc.)
                    let error = check_sandbox_permanent_failure(pod);

                    let phase = if is_terminating {
                        PodPhase::Terminating
                    } else if raw_phase == "Running" && !is_sandbox_container_ready(pod) {
                        PodPhase::Pending
                    } else {
                        PodPhase::from(raw_phase)
                    };

                    if phase == PodPhase::Running && error.is_none() {
                        let (phase, error) = self.running_or_unreachable(task_id).await;
                        return (phase, error, current_image);
                    }
                    (phase, error, current_image)
                } else {
                    // Deployment exists but no pods yet (scaling up)
                    (PodPhase::Pending, None, current_image)
                }
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to get pod status");
                (PodPhase::Unknown, None, current_image)
            }
        }
    }
}
