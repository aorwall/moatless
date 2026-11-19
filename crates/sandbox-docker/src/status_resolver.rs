//! Read-only domain service for resolving sandbox status from Docker.
//!
//! Maps Docker container states to `SandboxStatus`.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, warn};

use super::client::DockerClient;
use super::naming::generate_container_name;
use moatless_core::config::Config;
use moatless_sandbox_control::status::StatusResolve;
use moatless_sandbox_runtime::client::{AgentStatusSnapshot, SandboxConnector, fetch_agent_status};
use moatless_sandbox_runtime::live_status::{LiveStatus, derive_live_status};
use moatless_sandbox_runtime::schemas::*;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::sandbox::SandboxStatus;
use moatless_types::task::TaskStatus;

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
}

#[async_trait]
impl StatusResolve for DockerStatusResolver {
    async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
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
            } = fetch_agent_status(task_id, connector).await;
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

        SandboxStatusFullResponse {
            sandbox_status: live_status.sandbox_status,
            sandbox_error: live_status.sandbox_error,
            agent_status: live_status.agent_status,
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

    async fn resolve_live_status(
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

impl DockerStatusResolver {
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

                if max_restarts > 0 && restart_count as i32 >= max_restarts {
                    return (
                        SandboxStatus::Error,
                        Some(format!("Crash loop detected (restarts: {restart_count})")),
                    );
                }

                let status_str = state
                    .and_then(|s| s.status.as_ref())
                    .map_or_default(std::string::ToString::to_string);

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
