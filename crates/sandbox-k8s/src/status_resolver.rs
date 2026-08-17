//! Read-only domain service for resolving sandbox status.
//!
//! Resolves environment and server status directly from the K8s API.
//! Server entries are built config-first (server configs passed by caller)
//! with K8s container probe statuses overlaid.

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use std::sync::Arc;
use tracing::{debug, warn};

use super::client::K8sClient;
use super::deployment;
use super::pod_reader;
use super::pod_status::{
    check_sandbox_permanent_failure, is_sandbox_container_ready, pick_best_pod,
};
use super::service;
use moatless_backend_core::models::{SandboxDesiredState, TaskStatus};
use moatless_backend_core::sandbox::SandboxConnector;
use moatless_core::config::Config;
use moatless_sandbox_control::live_status::{AgentStatus, LiveStatus, derive_live_status};
use moatless_sandbox_control::models::DEFAULT_SERVER_READINESS_TIMEOUT_SECS;
use moatless_sandbox_control::models::*;
use moatless_sandbox_control::schemas::*;
use moatless_workspaces::repository::models::ServerConfig;

/// Snapshot of the agent's gRPC status response — state, scheduled jobs, and
/// live background processes.
struct AgentStatusSnapshot {
    status: Option<AgentStatus>,
    scheduled_jobs: Vec<ScheduledJobInfo>,
    background_processes: Vec<BackgroundProcess>,
}

impl AgentStatusSnapshot {
    /// The ready Sandbox's Agent is unreachable, so no raw Agent signal exists.
    fn unreachable() -> Self {
        Self {
            status: None,
            scheduled_jobs: Vec::new(),
            background_processes: Vec::new(),
        }
    }
}

/// Read-only domain service for resolving sandbox status from the K8s API.
///
/// Pure infrastructure — no database dependencies. Server configs are
/// provided by callers (resolved from DB in handlers).
pub struct StatusResolver {
    k8s: K8sClient,
    config: Arc<Config>,
}

impl StatusResolver {
    pub fn new(k8s: K8sClient, config: Arc<Config>) -> Self {
        Self { k8s, config }
    }

    /// Full sandbox status (env + servers) from K8s API.
    ///
    /// `server_configs` are the canonical server definitions from the DB.
    /// K8s container statuses are overlaid onto them.
    pub async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        server_configs: &[ServerConfig],
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
            // Config-first: start from server configs, overlay K8s container statuses
            let container_statuses =
                pod_reader::fetch_server_container_statuses(&self.k8s, sandbox_id).await;

            let servers: Vec<ServerEntry> = server_configs
                .iter()
                .map(|config| {
                    let container = container_statuses.iter().find(|cs| {
                        let name = cs.name.strip_suffix("-server").unwrap_or(&cs.name);
                        name == config.name
                    });
                    let timeout = config
                        .startup_timeout_seconds
                        .unwrap_or(DEFAULT_SERVER_READINESS_TIMEOUT_SECS);
                    let status = container.map_or(ServerStatus::Starting, |cs| {
                        cs.to_server_status_with_timeout(timeout)
                    });
                    let url = self.build_proxy_url(sandbox_id, config.port);
                    ServerEntry {
                        name: config.name.clone(),
                        label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                        port: config.port,
                        status,
                        url,
                        error: container.and_then(|cs| cs.error_reason_with_timeout(timeout)),
                        detail: container
                            .and_then(moatless_sandbox_control::models::ContainerObservedStatus::detail_reason),
                        default: config.default,
                        capabilities: server_capabilities_from_config(config),
                    }
                })
                .collect();

            // Query actual agent status from the sandbox pod via gRPC
            let AgentStatusSnapshot {
                status: agent_status,
                scheduled_jobs,
                background_processes,
            } = self.fetch_agent_status(task_id, connector).await;

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
                servers,
                resources: None,
                container_diagnostics,
                runtime_events: None,
                scheduled_jobs,
                background_processes,
                sandbox_desired_state: None,
                current_sandbox_image,
                latest_sandbox_image: None,
                image_up_to_date: None,
            };
        }

        // For Pending/Failed/Unknown phases, overlay K8s container statuses on config
        let servers = if matches!(
            phase,
            PodPhase::Pending | PodPhase::Failed | PodPhase::Unknown
        ) {
            let container_statuses =
                pod_reader::fetch_server_container_statuses(&self.k8s, sandbox_id).await;

            server_configs
                .iter()
                .map(|config| {
                    let container = container_statuses.iter().find(|cs| {
                        let name = cs.name.strip_suffix("-server").unwrap_or(&cs.name);
                        name == config.name
                    });
                    let timeout = config
                        .startup_timeout_seconds
                        .unwrap_or(DEFAULT_SERVER_READINESS_TIMEOUT_SECS);
                    let status = container.map_or(ServerStatus::Starting, |cs| {
                        cs.to_server_status_with_timeout(timeout)
                    });
                    let url = self.build_proxy_url(sandbox_id, config.port);
                    ServerEntry {
                        name: config.name.clone(),
                        label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                        port: config.port,
                        status,
                        url,
                        error: container.and_then(|cs| cs.error_reason_with_timeout(timeout)),
                        detail: container
                            .and_then(moatless_sandbox_control::models::ContainerObservedStatus::detail_reason),
                        default: config.default,
                        capabilities: server_capabilities_from_config(config),
                    }
                })
                .collect()
        } else {
            // Stopped/NotFound/Terminating/Succeeded: no pod is serving, so the
            // declared servers are listed as stopped. `Starting` here would
            // promise a page that is about to load and leave every client
            // spinning against an ingress with no backend.
            server_configs
                .iter()
                .map(|config| {
                    let url = self.build_proxy_url(sandbox_id, config.port);
                    ServerEntry {
                        name: config.name.clone(),
                        label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                        port: config.port,
                        status: ServerStatus::Stopped,
                        url,
                        error: None,
                        detail: None,
                        default: config.default,
                        capabilities: server_capabilities_from_config(config),
                    }
                })
                .collect()
        };

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
            servers,
            resources: None,
            container_diagnostics,
            runtime_events,
            scheduled_jobs: Vec::new(),
            background_processes: Vec::new(),
            sandbox_desired_state: None,
            current_sandbox_image,
            latest_sandbox_image: None,
            image_up_to_date: None,
        }
    }

    /// Resolve only the Task's live status, omitting servers and diagnostics.
    pub async fn resolve_live_status(
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
        let agent_status = if sandbox_status == SandboxStatus::Ready {
            self.fetch_agent_status(task_id, connector).await.status
        } else {
            None
        };

        derive_live_status(
            task_status,
            desired_state,
            sandbox_status,
            resolver_error,
            agent_status,
        )
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    /// Query the sandbox pod for current agent status via gRPC.
    ///
    /// Only called when the pod is running, so any gRPC error (timeout,
    /// unavailable, etc.) means the Agent is unreachable rather than idle.
    async fn fetch_agent_status(
        &self,
        task_id: &str,
        connector: &dyn SandboxConnector,
    ) -> AgentStatusSnapshot {
        let client = match connector.sandbox_client(task_id).await {
            Ok(c) => c,
            Err(e) => {
                debug!(task_id = %task_id, error = %e, "Failed to create gRPC client for agent status");
                return AgentStatusSnapshot::unreachable();
            }
        };

        match client.get_agent_status_for_task(Some(task_id)).await {
            Ok(resp) => AgentStatusSnapshot {
                status: Some(AgentStatus::from(resp.status.as_str())),
                scheduled_jobs: resp
                    .scheduled_jobs
                    .into_iter()
                    .filter_map(ScheduledJobInfo::from_proto)
                    .collect(),
                background_processes: resp
                    .background_processes
                    .into_iter()
                    .map(BackgroundProcess::from_proto)
                    .collect(),
            },
            Err(e) => {
                debug!(task_id = %task_id, error = %e, "Failed to fetch agent status via gRPC");
                AgentStatusSnapshot::unreachable()
            }
        }
    }

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

    /// Resolve the effective pod phase, any permanent error, and the image the
    /// sandbox container is provisioned with, checking Deployment replicas first.
    ///
    /// The third tuple element is the image baked into the Deployment's
    /// `sandbox` container (`None` when no Deployment exists). If a Deployment
    /// exists with replicas=0, returns `(PodPhase::Stopped, None, image)`.
    /// Otherwise falls back to checking actual pod status and probing for
    /// permanent failures (CrashLoopBackOff, ImagePullBackOff, etc.).
    ///
    /// `desired_state` is read only on the no-Deployment path, to tell a
    /// sandbox that never existed from one the cold transition reclaimed.
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

    /// Build a proxy URL for a server port using the configured proxy domain.
    fn build_proxy_url(&self, task_id: &str, port: u16) -> Option<String> {
        self.config.proxy.domain.as_ref().map(|domain| {
            format!(
                "{}://{}--{}.{}",
                self.config.proxy.protocol, task_id, port, domain
            )
        })
    }
}
