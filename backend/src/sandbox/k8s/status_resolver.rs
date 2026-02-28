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
use super::naming::generate_service_name;
use super::pod_status::{is_sandbox_container_ready, pick_best_pod};
use crate::config::Config;
use crate::sandbox::client::SandboxClient;
use crate::sandbox::models::*;
use crate::sandbox::schemas::*;

/// Read-only domain service for resolving sandbox status from the K8s API.
///
/// Pure infrastructure — no database dependencies. Server configs are
/// provided by callers (resolved from DB in handlers).
pub struct StatusResolver {
    k8s: K8sClient,
    config: Arc<Config>,
}

impl StatusResolver {
    pub fn new(
        k8s: K8sClient,
        config: Arc<Config>,
    ) -> Self {
        Self {
            k8s,
            config,
        }
    }

    /// Full sandbox status (env + servers) from K8s API.
    ///
    /// `server_configs` are the canonical server definitions from the DB.
    /// K8s container statuses are overlaid onto them.
    pub async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        server_configs: &[ServerConfig],
    ) -> SandboxStatusFullResponse {
        let phase = self.resolve_pod_phase(task_id).await;
        let environment_status = phase.to_environment_status();
        let is_running = phase == PodPhase::Running;

        if is_running {
            // Config-first: start from server configs, overlay K8s container statuses
            let container_statuses =
                pod_reader::fetch_server_container_statuses(&self.k8s, task_id).await;

            let servers: Vec<ServerEntry> = server_configs
                .iter()
                .map(|config| {
                    let container = container_statuses.iter().find(|cs| {
                        let name = cs.name.strip_suffix("-server").unwrap_or(&cs.name);
                        name == config.name
                    });
                    let status = container
                        .map_or(ServerStatus::Starting, ContainerObservedStatus::to_server_status);
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

            // Query actual agent status from the sandbox pod via gRPC
            let agent_state = self.fetch_agent_state(task_id).await;

            return SandboxStatusFullResponse {
                sandbox_state: SandboxState::Exists,
                environment_status,
                environment_error: None,
                agent_state,
                servers,
                sandbox_ready: true,
                can_execute: true,
                resources: None,
            };
        }

        // Pod is not running
        let sandbox_state = match phase {
            PodPhase::NotFound => SandboxState::Missing,
            _ => SandboxState::Exists,
        };

        let can_execute = phase == PodPhase::Stopped;

        // For Pending/Failed/Unknown phases, overlay K8s container statuses on config
        let servers = if matches!(
            phase,
            PodPhase::Pending | PodPhase::Failed | PodPhase::Unknown
        ) {
            let container_statuses =
                pod_reader::fetch_server_container_statuses(&self.k8s, task_id).await;

            server_configs
                .iter()
                .map(|config| {
                    let container = container_statuses.iter().find(|cs| {
                        let name = cs.name.strip_suffix("-server").unwrap_or(&cs.name);
                        name == config.name
                    });
                    let status = container
                        .map_or(ServerStatus::Starting, ContainerObservedStatus::to_server_status);
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
            // For Stopped/NotFound, show configs with Starting status as fallback
            server_configs
                .iter()
                .map(|config| {
                    let url = self.build_proxy_url(task_id, config.port);
                    ServerEntry {
                        name: config.name.clone(),
                        label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                        port: config.port,
                        status: ServerStatus::Starting,
                        url,
                        error: None,
                        detail: None,
                        default: config.default,
                        capabilities: server_capabilities_from_config(config),
                    }
                })
                .collect()
        };

        SandboxStatusFullResponse {
            sandbox_state,
            environment_status,
            environment_error: None,
            agent_state: AgentState::None,
            servers,
            sandbox_ready: false,
            can_execute,
            resources: None,
        }
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    /// Query the sandbox pod for current agent status via gRPC.
    ///
    /// Returns `AgentState::None` on any gRPC error (timeout, unavailable, etc.)
    /// so that status resolution remains resilient.
    async fn fetch_agent_state(&self, task_id: &str) -> AgentState {
        let service_name = generate_service_name(task_id);
        let client = match SandboxClient::for_service(
            &service_name,
            self.k8s.namespace(),
            self.config.sandbox.grpc_port,
        ) {
            Ok(c) => c,
            Err(e) => {
                debug!(task_id = %task_id, error = %e, "Failed to create gRPC client for agent status");
                return AgentState::None;
            }
        };

        match client.get_agent_status().await {
            Ok(resp) => AgentState::from(resp.status.as_str()),
            Err(e) => {
                debug!(task_id = %task_id, error = %e, "Failed to fetch agent status via gRPC");
                AgentState::None
            }
        }
    }

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
            // No Deployment found — sandbox doesn't exist
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
