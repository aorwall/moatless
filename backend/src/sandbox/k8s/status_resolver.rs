//! Read-only domain service for resolving sandbox status.
//!
//! Resolves environment and server status directly from the K8s API.
//! Server entries are built config-first (repo config from DB) with
//! K8s container probe statuses overlaid.

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use std::sync::Arc;
use tracing::warn;

use super::auto_provision;
use super::client::K8sClient;
use super::deployment;
use super::health;
use super::pod_status::{is_sandbox_container_ready, pick_best_pod};
use crate::config::Config;
use crate::repository::dao::RepoRepository;
use crate::task::dao::TaskRepository;
use crate::sandbox::models::*;
use crate::sandbox::schemas::*;

/// Read-only domain service for resolving sandbox status from the K8s API.
pub struct StatusResolver {
    k8s: K8sClient,
    config: Arc<Config>,
    tasks: Option<Arc<dyn TaskRepository>>,
    repos: Option<Arc<dyn RepoRepository>>,
}

impl StatusResolver {
    pub fn new(
        k8s: K8sClient,
        config: Arc<Config>,
        tasks: Option<Arc<dyn TaskRepository>>,
        repos: Option<Arc<dyn RepoRepository>>,
    ) -> Self {
        Self {
            k8s,
            config,
            tasks,
            repos,
        }
    }

    /// Lightweight env status from K8s API.
    pub async fn resolve_status(&self, task_id: &str) -> StatusResponse {
        let phase = self.resolve_pod_phase(task_id).await;
        let pod_name = if phase == PodPhase::Running || phase == PodPhase::Pending {
            self.get_task_pod_name(task_id).await
        } else {
            None
        };

        StatusResponse {
            status: phase.to_environment_status(),
            pod_name,
            last_error: None,
        }
    }

    /// Full sandbox status (env + servers) from K8s API.
    pub async fn resolve_sandbox_status(
        &self,
        task_id: &str,
    ) -> WorkspaceStatusFullResponse {
        let phase = self.resolve_pod_phase(task_id).await;
        let environment_status = phase.to_environment_status();
        let is_running = phase == PodPhase::Running;

        if is_running {
            // Config-first: start from repo config, overlay K8s container statuses
            let server_configs = self.get_server_configs(task_id).await;
            let container_statuses =
                health::fetch_server_container_statuses(&self.k8s, task_id).await;

            let servers: Vec<ServerEntry> = server_configs
                .iter()
                .map(|config| {
                    let container = container_statuses.iter().find(|cs| {
                        let name = cs.name.strip_suffix("-server").unwrap_or(&cs.name);
                        name == config.name
                    });
                    let status = container
                        .map_or(ServerStatus::Starting, |cs| ServerStatus::from(cs.to_server_status()));
                    let url = self.build_proxy_url(task_id, config.port);
                    ServerEntry {
                        name: config.name.clone(),
                        label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                        port: config.port,
                        status,
                        url,
                        error: container.and_then(super::super::models::ContainerObservedStatus::error_reason),
                        detail: container.and_then(super::super::models::ContainerObservedStatus::detail_reason),
                        default: config.default,
                        capabilities: server_capabilities_from_config(config),
                    }
                })
                .collect();

            return WorkspaceStatusFullResponse {
                workspace_state: WorkspaceState::Exists,
                environment_status,
                environment_error: None,
                agent_state: AgentState::None,
                servers,
                is_running: false,
                workspace_ready: true,
                can_execute: true,
                resources: None,
            };
        }

        // Pod is not running
        let workspace_state = match phase {
            PodPhase::NotFound => WorkspaceState::Missing,
            _ => WorkspaceState::Exists,
        };

        let can_execute = phase == PodPhase::Stopped;

        // For Pending/Failed/Unknown phases, overlay K8s container statuses on config
        let servers = if matches!(
            phase,
            PodPhase::Pending | PodPhase::Failed | PodPhase::Unknown
        ) {
            let server_configs = self.get_server_configs(task_id).await;
            let container_statuses =
                health::fetch_server_container_statuses(&self.k8s, task_id).await;

            server_configs
                .iter()
                .map(|config| {
                    let container = container_statuses.iter().find(|cs| {
                        let name = cs.name.strip_suffix("-server").unwrap_or(&cs.name);
                        name == config.name
                    });
                    let status = container
                        .map_or(ServerStatus::Starting, |cs| ServerStatus::from(cs.to_server_status()));
                    let url = self.build_proxy_url(task_id, config.port);
                    ServerEntry {
                        name: config.name.clone(),
                        label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                        port: config.port,
                        status,
                        url,
                        error: container.and_then(super::super::models::ContainerObservedStatus::error_reason),
                        detail: container.and_then(super::super::models::ContainerObservedStatus::detail_reason),
                        default: config.default,
                        capabilities: server_capabilities_from_config(config),
                    }
                })
                .collect()
        } else {
            self.servers_from_repo_config(task_id).await
        };

        WorkspaceStatusFullResponse {
            workspace_state,
            environment_status,
            environment_error: None,
            agent_state: AgentState::None,
            servers,
            is_running: false,
            workspace_ready: false,
            can_execute,
            resources: None,
        }
    }

    /// Direct K8s health check (no caching).
    pub async fn resolve_health(&self, task_id: &str) -> HealthResponse {
        let pods_api: Api<Pod> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp = ListParams::default()
            .labels(&format!("moatless.io/task-id={task_id}"));
        let healthy = match pods_api.list(&lp).await {
            Ok(list) => {
                pick_best_pod(&list.items).is_some_and(is_sandbox_container_ready)
            }
            Err(_) => false,
        };
        HealthResponse { healthy }
    }

    // =========================================================================
    // Private helpers (moved from K8sSandboxProvider)
    // =========================================================================

    /// Resolve the effective pod phase, checking Deployment replicas first.
    ///
    /// If a Deployment exists with replicas=0, returns `PodPhase::Stopped`.
    /// Otherwise falls back to checking actual pod status.
    async fn resolve_pod_phase(&self, task_id: &str) -> PodPhase {
        // Check Deployment replicas first
        if let Ok(Some(_dep)) = deployment::find_deployment(&self.k8s, task_id).await
        {
            if let Ok((desired, _ready)) =
                deployment::get_deployment_replicas(&self.k8s, task_id).await
            {
                if desired == 0 {
                    return PodPhase::Stopped;
                }
            }
        } else {
            // No Deployment found — workspace doesn't exist
            return PodPhase::NotFound;
        }

        // Deployment exists with replicas > 0, check actual pod status
        let pods_api: Api<Pod> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp = ListParams::default()
            .labels(&format!("moatless.io/task-id={task_id}"));

        match pods_api.list(&lp).await {
            Ok(list) => {
                if let Some(pod) = pick_best_pod(&list.items) {
                    let is_terminating =
                        pod.metadata.deletion_timestamp.is_some();
                    let raw_phase = pod
                        .status
                        .as_ref()
                        .and_then(|s| s.phase.as_deref())
                        .unwrap_or("Unknown");
                    if is_terminating {
                        PodPhase::Terminating
                    } else if raw_phase == "Running"
                        && !is_sandbox_container_ready(pod)
                    {
                        PodPhase::Pending
                    } else {
                        PodPhase::from(raw_phase)
                    }
                } else {
                    // Deployment exists but no pods yet (scaling up)
                    PodPhase::Pending
                }
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to get pod status");
                PodPhase::Unknown
            }
        }
    }

    /// Get the name of the first pod matching a task ID.
    async fn get_task_pod_name(&self, task_id: &str) -> Option<String> {
        let pods_api: Api<Pod> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp = ListParams::default()
            .labels(&format!("moatless.io/task-id={task_id}"));

        if let Ok(list) = pods_api.list(&lp).await {
            pick_best_pod(&list.items).and_then(|p| p.metadata.name.clone())
        } else {
            None
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

    /// Get the canonical server configs for a task from the DB.
    async fn get_server_configs(
        &self,
        task_id: &str,
    ) -> Vec<crate::sandbox::models::ServerConfig> {
        let (Some(ref tasks), Some(ref repos)) = (&self.tasks, &self.repos) else {
            return vec![];
        };

        match auto_provision::get_server_configs(tasks.as_ref(), repos.as_ref(), task_id).await {
            Ok(configs) => configs,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to get server configs");
                vec![]
            }
        }
    }

    /// Derive server entries from the repository config in the DB.
    ///
    /// Used as a fallback when K8s returns no container statuses (e.g. pod
    /// not yet scheduled). Returns configured servers with `Starting` status
    /// so the frontend shows them while the sandbox boots.
    async fn servers_from_repo_config(
        &self,
        task_id: &str,
    ) -> Vec<ServerEntry> {
        self.get_server_configs(task_id)
            .await
            .into_iter()
            .map(|s| {
                let url = self.build_proxy_url(task_id, s.port);
                ServerEntry {
                    label: s.label.clone().unwrap_or_else(|| s.name.clone()),
                    default: s.default,
                    capabilities: server_capabilities_from_config(&s),
                    name: s.name,
                    port: s.port,
                    status: ServerStatus::Starting,
                    url,
                    error: None,
                    detail: None,
                }
            })
            .collect()
    }
}
