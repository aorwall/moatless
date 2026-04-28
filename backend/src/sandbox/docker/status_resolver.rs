//! Read-only domain service for resolving sandbox status from Docker.
//!
//! Maps Docker container states to `SandboxStatus` and resolves
//! server sidecar container statuses via the Docker API.

use std::sync::Arc;
use tracing::{debug, warn};

use super::client::DockerClient;
use super::naming::{generate_container_name, generate_server_container_name};
use crate::config::Config;
use crate::sandbox::client::SandboxClient;
use crate::sandbox::k8s::pod_status::DEFAULT_SERVER_READINESS_TIMEOUT_SECS;
use crate::sandbox::models::*;
use crate::sandbox::schemas::*;

/// Snapshot of the agent's gRPC status response — state + scheduled jobs.
struct AgentStatusSnapshot {
    state: AgentState,
    scheduled_jobs: Vec<ScheduledJobInfo>,
}

impl AgentStatusSnapshot {
    fn empty() -> Self {
        Self {
            state: AgentState::None,
            scheduled_jobs: Vec::new(),
        }
    }
}

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
    ) -> SandboxStatusFullResponse {
        let container_name = generate_container_name(task_id);
        let (env_status, environment_error) =
            self.inspect_environment_status(&container_name).await;
        let is_running = env_status == SandboxStatus::Ready;

        if is_running {
            let AgentStatusSnapshot {
                state: agent_state,
                scheduled_jobs,
            } = self.fetch_agent_state(task_id).await;
            let servers = self.resolve_server_entries(task_id, server_configs).await;

            return SandboxStatusFullResponse {
                sandbox_state: SandboxState::Exists,
                environment_status: env_status,
                environment_error,
                agent_state,
                servers,
                sandbox_ready: true,
                can_execute: true,
                resources: None,
                container_diagnostics: None,
                runtime_events: None,
                scheduled_jobs,
            };
        }

        // Container is not running
        let sandbox_state = match env_status {
            SandboxStatus::NotCreated => SandboxState::Missing,
            _ => SandboxState::Exists,
        };

        let can_execute = env_status == SandboxStatus::Stopped;

        SandboxStatusFullResponse {
            sandbox_state,
            environment_status: env_status,
            environment_error,
            agent_state: AgentState::None,
            servers: vec![],
            sandbox_ready: false,
            can_execute,
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs: Vec::new(),
        }
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    /// Inspect a Docker container and map its state to `SandboxStatus`,
    /// along with an optional error message for crash loops or OOM.
    async fn inspect_environment_status(
        &self,
        container_name: &str,
    ) -> (SandboxStatus, Option<String>) {
        match self
            .client
            .docker
            .inspect_container(container_name, None)
            .await
        {
            Ok(info) => {
                let state = info.state.as_ref();
                let restart_count = info.restart_count.unwrap_or(0);
                let max_restarts = self.config.sandbox.max_crash_restart_count;

                // Check for OOM first
                if state.and_then(|s| s.oom_killed).unwrap_or(false) {
                    return (SandboxStatus::Error, Some("Container OOM killed".into()));
                }

                // Check crash loop via restart count
                if max_restarts > 0 && restart_count as i32 >= max_restarts {
                    return (
                        SandboxStatus::Error,
                        Some(format!("Crash loop detected (restarts: {restart_count})")),
                    );
                }

                let status_str = state
                    .and_then(|s| s.status.as_ref())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();

                let env_status = match status_str.as_str() {
                    "running" => SandboxStatus::Ready,
                    "created" | "restarting" => SandboxStatus::Initializing,
                    "paused" | "exited" => SandboxStatus::Stopped,
                    "dead" => SandboxStatus::Error,
                    "removing" => SandboxStatus::Removing,
                    _ => SandboxStatus::Error,
                };

                (env_status, None)
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => (SandboxStatus::NotCreated, None),
            Err(e) => {
                warn!(container = %container_name, error = %e, "Failed to inspect container");
                (SandboxStatus::Error, None)
            }
        }
    }

    /// Query the sandbox container for current agent status via gRPC.
    async fn fetch_agent_state(&self, task_id: &str) -> AgentStatusSnapshot {
        let container_name = generate_container_name(task_id);
        let url = format!(
            "http://{}:{}",
            container_name, self.config.sandbox.grpc_port
        );
        let client = match SandboxClient::connect(&url) {
            Ok(c) => c,
            Err(e) => {
                debug!(task_id = %task_id, error = %e, "Failed to create gRPC client for agent status");
                return AgentStatusSnapshot::empty();
            }
        };

        match client.get_agent_status().await {
            Ok(resp) => AgentStatusSnapshot {
                state: AgentState::from(resp.status.as_str()),
                scheduled_jobs: resp
                    .scheduled_jobs
                    .into_iter()
                    .filter_map(ScheduledJobInfo::from_proto)
                    .collect(),
            },
            Err(e) => {
                debug!(task_id = %task_id, error = %e, "Failed to fetch agent status via gRPC");
                AgentStatusSnapshot::empty()
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
            let timeout = sc
                .startup_timeout_seconds
                .unwrap_or(DEFAULT_SERVER_READINESS_TIMEOUT_SECS);
            let (status, url, error) = self
                .inspect_server_container(&container_name, sc.port, timeout)
                .await;

            entries.push(ServerEntry {
                name: sc.name.clone(),
                label: sc.label.clone().unwrap_or_else(|| sc.name.clone()),
                port: sc.port,
                status,
                url,
                error,
                detail: None,
                default: sc.default,
                capabilities: server_capabilities_from_config(sc),
            });
        }

        entries
    }

    /// Inspect a server container and return its status, host URL, and optional error.
    ///
    /// When a container is "running", checks whether the port is actually
    /// reachable via TCP. If not, and the container has been running longer
    /// than `startup_timeout_secs`, returns `Failed`.
    async fn inspect_server_container(
        &self,
        container_name: &str,
        container_port: u16,
        startup_timeout_secs: u32,
    ) -> (ServerStatus, Option<String>, Option<String>) {
        match self
            .client
            .docker
            .inspect_container(container_name, None)
            .await
        {
            Ok(info) => {
                let state = info.state.as_ref();
                let status_str = state
                    .and_then(|s| s.status.as_ref())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();
                let restart_count = info.restart_count.unwrap_or(0);
                let max_restarts = self.config.sandbox.max_crash_restart_count;

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

                let url = host_port
                    .as_ref()
                    .map(|port| format!("http://localhost:{port}"));

                // Detect crash loop (applies to both "running" and "restarting"
                // statuses) — a container that's restarting past the threshold
                // is failing; Docker's "restarting" status otherwise masks it
                // as Starting forever.
                if max_restarts > 0 && restart_count as i32 >= max_restarts {
                    let exit_code = state.and_then(|s| s.exit_code).unwrap_or(0);
                    let state_error = state
                        .and_then(|s| s.error.as_ref())
                        .filter(|e| !e.is_empty())
                        .cloned();
                    let msg = match state_error {
                        Some(e) => format!(
                            "Crash loop (restarts: {restart_count}, exit {exit_code}): {e}"
                        ),
                        None => format!("Crash loop (restarts: {restart_count}, exit {exit_code})"),
                    };
                    return (ServerStatus::Failed, url, Some(msg));
                }

                let server_status = match status_str.as_str() {
                    "running" => {
                        // Check if the port is actually reachable via the shared
                        // Docker network (container name → internal port). The
                        // status resolver runs inside the backend container, so
                        // `127.0.0.1:host_port` wouldn't work — that's the host
                        // machine's loopback, not reachable from here.
                        let port_open = tokio::net::TcpStream::connect((
                            container_name,
                            container_port,
                        ))
                        .await
                        .is_ok();

                        if port_open {
                            ServerStatus::Started
                        } else {
                            // Port not open — check if we've exceeded the startup timeout
                            let started_at = info
                                .state
                                .as_ref()
                                .and_then(|s| s.started_at.as_deref())
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                .map(|dt| dt.with_timezone(&chrono::Utc));

                            if let Some(started) = started_at {
                                let elapsed = chrono::Utc::now() - started;
                                if elapsed > chrono::Duration::seconds(startup_timeout_secs as i64)
                                {
                                    return (
                                        ServerStatus::Failed,
                                        url,
                                        Some(
                                            "Server failed to become ready within timeout"
                                                .to_string(),
                                        ),
                                    );
                                }
                            }
                            ServerStatus::Starting
                        }
                    }
                    "created" | "restarting" => ServerStatus::Starting,
                    "dead" => ServerStatus::Failed,
                    "exited" => {
                        let exit_code = state.and_then(|s| s.exit_code).unwrap_or(0);
                        if exit_code == 0 {
                            ServerStatus::Stopped
                        } else {
                            return (
                                ServerStatus::Failed,
                                url,
                                Some(format!("Container exited with code {exit_code}")),
                            );
                        }
                    }
                    _ => ServerStatus::Stopped,
                };

                (server_status, url, None)
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                debug!(container = %container_name, "Server container not found");
                (ServerStatus::Stopped, None, None)
            }
            Err(e) => {
                warn!(container = %container_name, error = %e, "Failed to inspect server container");
                (ServerStatus::Failed, None, None)
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
