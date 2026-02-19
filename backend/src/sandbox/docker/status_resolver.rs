//! Read-only domain service for resolving sandbox status from Docker.
//!
//! Maps Docker container states to `EnvironmentStatus`. Docker mode is
//! agent-only — no server management, so servers are always empty.

use std::sync::Arc;
use tracing::{debug, warn};

use super::client::DockerClient;
use super::naming::generate_container_name;
use crate::config::Config;
use crate::sandbox::grpc_client::SandboxGrpcClient;
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

    /// Lightweight env status from Docker API.
    pub async fn resolve_status(&self, task_id: &str) -> StatusResponse {
        let container_name = generate_container_name(task_id);
        let env_status = self.inspect_environment_status(&container_name).await;

        StatusResponse {
            status: env_status,
            pod_name: None,
            last_error: None,
        }
    }

    /// Full sandbox status (env + agent state) from Docker API.
    pub async fn resolve_sandbox_status(
        &self,
        task_id: &str,
    ) -> WorkspaceStatusFullResponse {
        let container_name = generate_container_name(task_id);
        let env_status = self.inspect_environment_status(&container_name).await;
        let is_running = env_status == EnvironmentStatus::Ready;

        if is_running {
            let agent_state = self.fetch_agent_state(task_id).await;

            return WorkspaceStatusFullResponse {
                workspace_state: WorkspaceState::Exists,
                environment_status: env_status,
                environment_error: None,
                agent_state,
                servers: vec![],
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

    /// Direct Docker health check.
    pub async fn resolve_health(&self, task_id: &str) -> HealthResponse {
        let container_name = generate_container_name(task_id);
        let status = self.inspect_environment_status(&container_name).await;
        HealthResponse {
            healthy: status == EnvironmentStatus::Ready,
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
        let client = match SandboxGrpcClient::connect(&url) {
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
}
