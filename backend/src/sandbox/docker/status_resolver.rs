//! Read-only domain service for resolving sandbox status from Docker.
//!
//! Maps Docker container states to `EnvironmentStatus` and resolves
//! server sidecar container statuses via the Docker API.

use std::sync::Arc;
use tracing::{debug, warn};

use super::client::DockerClient;
use super::naming::{generate_container_name, generate_server_container_name};
use crate::config::Config;
use crate::sandbox::client::SandboxClient;
use crate::sandbox::models::*;
use crate::sandbox::schemas::*;

/// Read-only domain service for resolving sandbox status from Docker.
pub struct DockerStatusResolver {
    client: DockerClient,
    config: Arc<Config>,
}

impl DockerStatusResolver {
    pub fn new(client: DockerClient, config: Arc<Config>) -> Self {
        Self { client, config }
    }

    /// Full sandbox status (env + agent + servers) from Docker API.
    ///
    /// `server_configs` are the canonical server definitions from the DB.
    /// When non-empty, Docker container statuses are inspected and overlaid.
    pub async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        server_configs: &[ServerConfig],
    ) -> WorkspaceStatusFullResponse {
        let container_name = generate_container_name(task_id);
        let env_status = self.inspect_environment_status(&container_name).await;
        let is_running = env_status == EnvironmentStatus::Ready;

        if is_running {
            let agent_state = self.fetch_agent_state(task_id).await;
            let servers = self.resolve_server_entries(task_id, server_configs).await;

            return WorkspaceStatusFullResponse {
                workspace_state: WorkspaceState::Exists,
                environment_status: env_status,
                environment_error: None,
                agent_state,
                servers,
                workspace_ready: true,
                can_execute: true,
                resources: None,
            };
        }

        // Container is not running
        let workspace_state = match env_status {
            EnvironmentStatus::NotCreated => WorkspaceState::Missing,
            _ => WorkspaceState::Exists,
        };

        let can_execute = env_status == EnvironmentStatus::Stopped;

        WorkspaceStatusFullResponse {
            workspace_state,
            environment_status: env_status,
            environment_error: None,
            agent_state: AgentState::None,
            servers: vec![],
            workspace_ready: false,
            can_execute,
            resources: None,
        }
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    /// Inspect a Docker container and map its state to `EnvironmentStatus`.
    async fn inspect_environment_status(&self, container_name: &str) -> EnvironmentStatus {
        match self.client.docker.inspect_container(container_name, None).await {
            Ok(info) => {
                let state = info.state.as_ref();
                let status_str = state
                    .and_then(|s| s.status.as_ref())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();

                // Check for OOM first
                if state.and_then(|s| s.oom_killed).unwrap_or(false) {
                    return EnvironmentStatus::Error;
                }

                match status_str.as_str() {
                    "running" => EnvironmentStatus::Ready,
                    "created" | "restarting" => EnvironmentStatus::Initializing,
                    "paused" | "exited" => EnvironmentStatus::Stopped,
                    "dead" => EnvironmentStatus::Error,
                    "removing" => EnvironmentStatus::Removing,
                    _ => EnvironmentStatus::Error,
                }
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => EnvironmentStatus::NotCreated,
            Err(e) => {
                warn!(container = %container_name, error = %e, "Failed to inspect container");
                EnvironmentStatus::Error
            }
        }
    }

    /// Query the sandbox container for current agent status via gRPC.
    async fn fetch_agent_state(&self, task_id: &str) -> AgentState {
        let container_name = generate_container_name(task_id);
        let url = format!(
            "http://{}:{}",
            container_name, self.config.sandbox.grpc_port
        );
        let client = match SandboxClient::connect(&url) {
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

    /// Build server entries by inspecting Docker sidecar containers.
    ///
    /// For each `ServerConfig`, inspects the corresponding Docker container
    /// and extracts status + dynamically mapped host port.
    async fn resolve_server_entries(
        &self,
        task_id: &str,
        server_configs: &[ServerConfig],
    ) -> Vec<ServerEntry> {
        let mut entries = Vec::with_capacity(server_configs.len());

        for sc in server_configs {
            let container_name = generate_server_container_name(task_id, &sc.name);
            let (status, url) = self.inspect_server_container(&container_name, sc.port).await;

            entries.push(ServerEntry {
                name: sc.name.clone(),
                label: sc.label.clone().unwrap_or_else(|| sc.name.clone()),
                port: sc.port,
                status,
                url,
                error: None,
                detail: None,
                default: sc.default,
                capabilities: server_capabilities_from_config(sc),
            });
        }

        entries
    }

    /// Inspect a server container and return its status + host URL.
    async fn inspect_server_container(
        &self,
        container_name: &str,
        container_port: u16,
    ) -> (ServerStatus, Option<String>) {
        match self.client.docker.inspect_container(container_name, None).await {
            Ok(info) => {
                let status_str = info
                    .state
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();

                let server_status = match status_str.as_str() {
                    "running" => ServerStatus::Started,
                    "created" | "restarting" => ServerStatus::Starting,
                    "dead" => ServerStatus::Failed,
                    _ => ServerStatus::Stopped,
                };

                // Extract the dynamically assigned host port
                let host_port = info
                    .network_settings
                    .as_ref()
                    .and_then(|ns| ns.ports.as_ref())
                    .and_then(|ports| ports.get(&format!("{container_port}/tcp")))
                    .and_then(|bindings| bindings.as_ref())
                    .and_then(|bindings| bindings.first())
                    .and_then(|b| b.host_port.as_ref())
                    .and_then(|p| if p.is_empty() { None } else { Some(p.clone()) });

                let url = host_port.map(|port| format!("http://localhost:{port}"));

                (server_status, url)
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                debug!(container = %container_name, "Server container not found");
                (ServerStatus::Stopped, None)
            }
            Err(e) => {
                warn!(container = %container_name, error = %e, "Failed to inspect server container");
                (ServerStatus::Failed, None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_name_generation() {
        assert_eq!(
            generate_container_name("task-123"),
            "moatless-sandbox-task-123"
        );
    }

    #[test]
    fn server_container_name_generation() {
        assert_eq!(
            generate_server_container_name("task-123", "preview"),
            "moatless-server-task-123-preview"
        );
    }
}
