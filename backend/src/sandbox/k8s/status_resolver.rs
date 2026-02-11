//! Read-only domain service for resolving sandbox status.
//!
//! Reads from Redis (primary) with K8s API fallback. Never writes to Redis —
//! the pod_watcher is the sole writer (single-writer pattern).

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
use crate::db::DbPool;
use crate::sandbox::sandbox_state::SandboxStateCache;
use crate::sandbox::types::*;

/// Read-only domain service for resolving sandbox status.
///
/// Reads from Redis (primary) with K8s API fallback. Never writes to Redis —
/// the pod_watcher is the sole writer (single-writer pattern).
pub struct StatusResolver {
    k8s: K8sClient,
    sandbox_state: Option<SandboxStateCache>,
    config: Arc<Config>,
    db_pool: Option<DbPool>,
}

impl StatusResolver {
    pub fn new(
        k8s: K8sClient,
        sandbox_state: Option<SandboxStateCache>,
        config: Arc<Config>,
        db_pool: Option<DbPool>,
    ) -> Self {
        Self {
            k8s,
            sandbox_state,
            config,
            db_pool,
        }
    }

    /// Lightweight env status: Redis → K8s fallback.
    pub async fn resolve_status(&self, task_id: &str) -> StatusResponse {
        // Check Redis cache
        if let Some(ref state_cache) = self.sandbox_state {
            if let Some(cached) = state_cache.get_if_fresh(task_id).await {
                return StatusResponse {
                    status: cached.environment_status,
                    pod_name: cached.pod_name,
                    last_error: cached.environment_error,
                };
            }
        }

        // K8s API fallback
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

    /// Full sandbox status (env + servers + agent): Redis → K8s fallback.
    pub async fn resolve_sandbox_status(
        &self,
        task_id: &str,
    ) -> WorkspaceStatusFullResponse {
        // Check Redis cache for fresh full state
        if let Some(ref state_cache) = self.sandbox_state {
            if let Some(cached) = state_cache.get_if_fresh(task_id).await {
                let servers = cached
                    .servers
                    .iter()
                    .map(|s| {
                        let url = self.build_proxy_url(task_id, s.port);
                        ServerEntry {
                            name: s.name.clone(),
                            label: s.label.clone(),
                            port: s.port,
                            status: ServerStatus::from(s.status.as_str()),
                            url,
                            error: s.error.clone(),
                            default: s.name == "preview",
                            capabilities: get_server_capabilities(&s.name),
                        }
                    })
                    .collect();

                let workspace_ready =
                    cached.environment_status == EnvironmentStatus::Ready;
                let can_execute = workspace_ready
                    || cached.environment_status == EnvironmentStatus::Stopped;

                return WorkspaceStatusFullResponse {
                    workspace_state: cached.workspace_state,
                    environment_status: cached.environment_status,
                    environment_error: cached.environment_error,
                    agent_state: cached.agent_state,
                    servers,
                    is_running: cached.is_running,
                    workspace_ready,
                    can_execute,
                    resources: cached.resources,
                };
            }
        }

        // K8s API fallback
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
                        .map(|cs| ServerStatus::from(cs.to_server_status()))
                        .unwrap_or(ServerStatus::Starting);
                    let url = self.build_proxy_url(task_id, config.port);
                    ServerEntry {
                        name: config.name.clone(),
                        label: config.name.clone(),
                        port: config.port,
                        status,
                        url,
                        error: container.and_then(|cs| cs.waiting_reason.clone()),
                        default: config.name == "preview",
                        capabilities: get_server_capabilities(&config.name),
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
                        .map(|cs| ServerStatus::from(cs.to_server_status()))
                        .unwrap_or(ServerStatus::Starting);
                    let url = self.build_proxy_url(task_id, config.port);
                    ServerEntry {
                        name: config.name.clone(),
                        label: config.name.clone(),
                        port: config.port,
                        status,
                        url,
                        error: container.and_then(|cs| cs.waiting_reason.clone()),
                        default: config.name == "preview",
                        capabilities: get_server_capabilities(&config.name),
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
        self.config.proxy_domain.as_ref().map(|domain| {
            format!(
                "{}://{}--{}.{}",
                self.config.proxy_protocol, task_id, port, domain
            )
        })
    }

    /// Get the canonical server configs for a task from the DB.
    async fn get_server_configs(
        &self,
        task_id: &str,
    ) -> Vec<crate::sandbox::types::ServerConfig> {
        let Some(ref pool) = self.db_pool else {
            return vec![];
        };

        match auto_provision::get_server_configs(pool, task_id).await {
            Ok(configs) => configs,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to get server configs");
                vec![]
            }
        }
    }

    /// Derive server entries from the repository config in the DB.
    ///
    /// Used as a last-resort fallback when there's no Redis cache and K8s
    /// returns no container statuses. Returns configured servers with `Starting`
    /// status so the frontend shows them while the sandbox boots.
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
                    label: s.name.clone(),
                    default: s.name == "preview",
                    capabilities: get_server_capabilities(&s.name),
                    name: s.name,
                    port: s.port,
                    status: ServerStatus::Starting,
                    url,
                    error: None,
                }
            })
            .collect()
    }
}
