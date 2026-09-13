//! Read-only domain service for resolving sandbox status from Docker.
//!
//! Maps Docker container states to `SandboxStatus` and resolves server sidecar
//! container statuses via the Docker API.

use std::sync::Arc;
use tracing::{debug, warn};

use super::client::DockerClient;
use super::naming::{generate_container_name, generate_server_container_name};
use moatless_core::config::Config;
use moatless_sandbox_runtime::client::{
    SandboxConnector, managed_command_from_proto, scheduled_job_info_from_proto,
};
use moatless_sandbox_runtime::container_status::DEFAULT_SERVER_READINESS_TIMEOUT_SECS;
use moatless_sandbox_runtime::live_status::{AgentStatus, LiveStatus, derive_live_status};
use moatless_sandbox_runtime::provisioning::{allocated_port, server_capabilities_from_config};
use moatless_sandbox_runtime::schemas::*;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::sandbox::{SandboxStatus, ServerStatus};
use moatless_types::task::TaskStatus;
use moatless_workspace_types::workspace::ServerConfig;

/// Snapshot of the agent's gRPC status response — state, scheduled jobs, and
/// live background processes.
struct AgentStatusSnapshot {
    status: Option<AgentStatus>,
    scheduled_jobs: Vec<ScheduledJobInfo>,
    commands: Vec<ManagedCommand>,
}

impl AgentStatusSnapshot {
    fn unreachable() -> Self {
        Self {
            status: None,
            scheduled_jobs: Vec::new(),
            commands: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerContainerState {
    Created,
    Running,
    Restarting,
    Paused,
    Exited,
    Dead,
    Removing,
}

impl DockerContainerState {
    fn sandbox_status(self) -> SandboxStatus {
        match self {
            Self::Running => SandboxStatus::Ready,
            Self::Created | Self::Restarting => SandboxStatus::Initializing,
            Self::Paused | Self::Exited => SandboxStatus::Stopped,
            Self::Dead => SandboxStatus::Error,
            Self::Removing => SandboxStatus::Removing,
        }
    }
}

impl TryFrom<&str> for DockerContainerState {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "created" => Ok(Self::Created),
            "running" => Ok(Self::Running),
            "restarting" => Ok(Self::Restarting),
            "paused" => Ok(Self::Paused),
            "exited" => Ok(Self::Exited),
            "dead" => Ok(Self::Dead),
            "removing" => Ok(Self::Removing),
            other => Err(format!("unknown Docker container state: {other}")),
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
    /// `server_configs` are the canonical server definitions from the DB; when
    /// non-empty, Docker container statuses are inspected and overlaid.
    pub async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        server_configs: &[ServerConfig],
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> SandboxStatusFullResponse {
        let container_name = generate_container_name(sandbox_id);
        let (sandbox_status, resolver_error) = self.inspect_sandbox_status(&container_name).await;
        let current_sandbox_image = self.inspect_current_image(&container_name).await;
        let is_running = sandbox_status == SandboxStatus::Ready;

        if is_running {
            let AgentStatusSnapshot {
                status: agent_status,
                scheduled_jobs,
                commands,
            } = self.fetch_agent_status(task_id, connector).await;
            let servers = self
                .resolve_server_entries(sandbox_id, server_configs)
                .await;
            let live_status = derive_live_status(
                task_status,
                desired_state,
                sandbox_status,
                resolver_error,
                agent_status,
            );

            return SandboxStatusFullResponse {
                sandbox_status: live_status.sandbox_status,
                sandbox_error: live_status.sandbox_error,
                agent_status: live_status.agent_status,
                servers,
                resources: None,
                container_diagnostics: None,
                runtime_events: None,
                scheduled_jobs,
                commands,
                sandbox_desired_state: None,
                current_sandbox_image,
                latest_sandbox_image: None,
                image_up_to_date: None,
            };
        }

        let live_status = derive_live_status(
            task_status,
            desired_state,
            sandbox_status,
            resolver_error,
            None,
        );

        // The Task's declared servers are listed even with nothing running, so
        // a client can show what the Task declares rather than an empty panel.
        // No container is inspected: the sandbox is not running, so every one
        // of them is stopped, and the exit code a torn-down server container
        // left behind is not a failure of that server.
        let servers = server_configs
            .iter()
            .filter_map(|config| {
                Some(ServerEntry {
                    name: config.name.clone(),
                    label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                    port: allocated_port(config, task_id)?,
                    status: ServerStatus::Stopped,
                    url: None,
                    error: None,
                    detail: None,
                    default: config.default,
                    capabilities: server_capabilities_from_config(config),
                })
            })
            .collect();

        SandboxStatusFullResponse {
            sandbox_status: live_status.sandbox_status,
            sandbox_error: live_status.sandbox_error,
            agent_status: live_status.agent_status,
            servers,
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs: Vec::new(),
            commands: Vec::new(),
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
        let container_name = generate_container_name(sandbox_id);
        let (sandbox_status, resolver_error) = self.inspect_sandbox_status(&container_name).await;
        // One gRPC call answers both halves. Taking only `status` here is what
        // left a reader unable to see that `moat cmd` work was still running
        // behind an idle agent.
        let (agent_status, commands) = if sandbox_status == SandboxStatus::Ready {
            let snapshot = self.fetch_agent_status(task_id, connector).await;
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

    /// Inspect the sandbox container and return the image it is running.
    ///
    /// Best-effort — returns `None` when the container is absent or its image
    /// can't be read.
    async fn inspect_current_image(&self, container_name: &str) -> Option<String> {
        match self
            .client
            .docker
            .inspect_container(container_name, None)
            .await
        {
            Ok(info) => info.config.and_then(|c| c.image),
            Err(e) => {
                debug!(container = %container_name, error = %e, "Failed to inspect sandbox image");
                None
            }
        }
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    /// Inspect a Docker container and map its state to `SandboxStatus`, along
    /// with an optional error message for crash loops or OOM.
    async fn inspect_sandbox_status(
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

                let sandbox_status = DockerContainerState::try_from(status_str.as_str())
                    .map_or(SandboxStatus::Error, DockerContainerState::sandbox_status);

                (sandbox_status, None)
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
                    .filter_map(scheduled_job_info_from_proto)
                    .collect(),
                commands: resp
                    .commands
                    .into_iter()
                    .map(managed_command_from_proto)
                    .collect(),
            },
            Err(e) => {
                debug!(task_id = %task_id, error = %e, "Failed to fetch agent status via gRPC");
                AgentStatusSnapshot::unreachable()
            }
        }
    }

    /// Build server entries by inspecting Docker sidecar containers.
    ///
    /// For each `ServerConfig`, inspects the corresponding Docker container and
    /// extracts status + dynamically mapped host port.
    async fn resolve_server_entries(
        &self,
        task_id: &str,
        server_configs: &[ServerConfig],
    ) -> Vec<ServerEntry> {
        let mut entries = Vec::with_capacity(server_configs.len());

        for sc in server_configs {
            let Some(port) = allocated_port(sc, task_id) else {
                continue;
            };
            let container_name = generate_server_container_name(task_id, &sc.name);
            let timeout = sc
                .startup_timeout_seconds
                .unwrap_or(DEFAULT_SERVER_READINESS_TIMEOUT_SECS);
            let (status, url, error) = self
                .inspect_server_container(&container_name, port, timeout)
                .await;

            entries.push(ServerEntry {
                name: sc.name.clone(),
                label: sc.label.clone().unwrap_or_else(|| sc.name.clone()),
                port,
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

    /// Inspect a server container and return its status, host URL, and optional
    /// error.
    ///
    /// When "running", checks whether the port is reachable via TCP; if not and
    /// the container has run longer than `startup_timeout_secs`, returns
    /// `Failed`.
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
                        Some(e) => {
                            format!("Crash loop (restarts: {restart_count}, exit {exit_code}): {e}")
                        }
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
                        let port_open =
                            tokio::net::TcpStream::connect((container_name, container_port))
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

    #[test]
    fn docker_container_state_projects_to_sandbox_status() {
        let cases = [
            ("created", SandboxStatus::Initializing),
            ("running", SandboxStatus::Ready),
            ("restarting", SandboxStatus::Initializing),
            ("paused", SandboxStatus::Stopped),
            ("exited", SandboxStatus::Stopped),
            ("dead", SandboxStatus::Error),
            ("removing", SandboxStatus::Removing),
        ];

        for (state, expected) in cases {
            let state = DockerContainerState::try_from(state).unwrap();
            assert_eq!(state.sandbox_status(), expected, "{state:?}");
        }
    }

    #[test]
    fn docker_container_state_rejects_unknown_values() {
        assert!(DockerContainerState::try_from("").is_err());
        assert!(DockerContainerState::try_from("unknown").is_err());
    }
}
