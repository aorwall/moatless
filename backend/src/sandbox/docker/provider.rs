//! Docker implementation of the SandboxProvider trait.
//!
//! Manages sandbox lifecycle through Docker containers running as siblings
//! on a shared Docker network. Communication with sandbox containers uses
//! gRPC, resolved by container name as hostname on the Docker network.

use async_trait::async_trait;
use axum::response::sse::Event;
use bollard::container::{
    RemoveContainerOptions, RestartContainerOptions, StartContainerOptions, StopContainerOptions,
};
use futures::stream::BoxStream;
use std::sync::Arc;
use tracing::{info, warn};

use super::client::DockerClient;
use super::container_builder;
use super::naming::generate_container_name;
use super::status_resolver::DockerStatusResolver;
use crate::config::Config;
use crate::events::EventPublisher;
use crate::sandbox::error::SandboxError;
use crate::sandbox::client::SandboxClient;
use crate::sandbox::jwt;
use crate::sandbox::models::*;
use crate::sandbox::provider::SandboxProvider;
use crate::sandbox::schemas::*;

/// Docker-based sandbox provider.
///
/// Pure infrastructure provider — no database dependencies.
/// DB concerns (server configs, auto-provisioning) are handled by callers.
pub struct DockerSandboxProvider {
    pub config: Arc<Config>,
    pub client: DockerClient,
    pub status_resolver: DockerStatusResolver,
    /// Optional event publisher for environment lifecycle events.
    pub event_publisher: Option<Arc<dyn EventPublisher>>,
}

#[async_trait]
impl SandboxProvider for DockerSandboxProvider {
    async fn provision(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError> {
        info!(task_id = %task_id, user_id = %request.user_id, "Provisioning Docker sandbox");

        let container_name = generate_container_name(task_id);
        let plan = &request.plan;

        // 1. Check for existing container — idempotent, never force-remove
        match self.check_existing_container(&container_name).await {
            ExistingContainerResult::Running => {
                info!(task_id = %task_id, "Reusing existing running container");
                return Ok(ProvisionResponse {
                    status: EnvironmentStatus::Ready,
                });
            }
            ExistingContainerResult::Exists => {
                // Container exists but not running — start it
                info!(task_id = %task_id, "Starting existing stopped container");
                self.publish_initializing(task_id).await;
                self.client
                    .docker
                    .start_container(&container_name, None::<StartContainerOptions<String>>)
                    .await
                    .map_err(|e| SandboxError::DockerError(format!("Failed to start container: {e}")))?;

                self.publish_ready(task_id).await;

                return Ok(ProvisionResponse {
                    status: EnvironmentStatus::Ready,
                });
            }
            ExistingContainerResult::NotFound => {} // Continue provisioning
        }

        // 2. Generate JWT using user_id directly (no DB lookup)
        let task_token_secret = self
            .config
            .sandbox
            .task_token_secret
            .as_ref()
            .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?
            .clone();

        let user_auth_token =
            jwt::generate_user_auth_token(&request.user_id, &request.user_id, &task_token_secret)
                .map_err(|e| SandboxError::JwtError(e.to_string()))?;

        // 3. Create workspace directory on host
        let workspace_path =
            super::naming::generate_workspace_host_path(&self.config.docker.data_dir, task_id);
        // Use container_data_dir for creating the directory when running inside a container
        let container_workspace_path = super::naming::generate_workspace_host_path(
            &self.config.docker.container_data_dir,
            task_id,
        );
        if let Err(e) = tokio::fs::create_dir_all(&container_workspace_path).await {
            warn!(
                task_id = %task_id,
                path = %container_workspace_path,
                error = %e,
                "Failed to create workspace directory (may already exist)"
            );
        }

        // 4. Build container config
        let container_config = container_builder::build_container_config(
            &self.config,
            task_id,
            plan,
            Some(&user_auth_token),
        );

        // 5. Create and start container
        self.client
            .docker
            .create_container(
                Some(bollard::container::CreateContainerOptions {
                    name: &container_name,
                    platform: None,
                }),
                container_config,
            )
            .await
            .map_err(|e| SandboxError::DockerError(format!("Failed to create container: {e}")))?;

        // Publish initializing event before starting
        self.publish_initializing(task_id).await;

        self.client
            .docker
            .start_container(&container_name, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| SandboxError::DockerError(format!("Failed to start container: {e}")))?;

        info!(
            task_id = %task_id,
            container = %container_name,
            workspace_path = %workspace_path,
            "Docker sandbox provisioned"
        );

        self.publish_ready(task_id).await;

        Ok(ProvisionResponse {
            status: EnvironmentStatus::Ready,
        })
    }

    async fn sandbox_status(
        &self,
        task_id: &str,
        _server_configs: &[ServerConfig],
    ) -> Result<WorkspaceStatusFullResponse, SandboxError> {
        // Docker mode is agent-only — server_configs are ignored
        Ok(self.status_resolver.resolve_sandbox_status(task_id).await)
    }

    async fn stop(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Stopping Docker sandbox");

        let container_name = generate_container_name(task_id);

        self.client
            .docker
            .stop_container(
                &container_name,
                Some(StopContainerOptions { t: 30 }),
            )
            .await
            .map_err(|e| {
                SandboxError::DockerError(format!("Failed to stop container: {e}"))
            })?;

        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_sandbox_event(
                    task_id,
                    moatless_events::SandboxSubType::Stopped,
                    "Stopped",
                )
                .await;
        }

        Ok(())
    }

    async fn restart(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Restarting Docker sandbox");

        let container_name = generate_container_name(task_id);

        self.publish_initializing(task_id).await;

        self.client
            .docker
            .restart_container(&container_name, Some(RestartContainerOptions { t: 30 }))
            .await
            .map_err(|e| {
                SandboxError::DockerError(format!("Failed to restart container: {e}"))
            })?;

        self.publish_ready(task_id).await;

        Ok(())
    }

    async fn cleanup(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Cleaning up Docker sandbox");

        let container_name = generate_container_name(task_id);

        // Force remove container (stops it if running)
        if let Err(e) = self
            .client
            .docker
            .remove_container(
                &container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true, // Remove anonymous volumes
                    ..Default::default()
                }),
            )
            .await
        {
            // 404 is fine — container already gone
            if !matches!(
                e,
                bollard::errors::Error::DockerResponseServerError {
                    status_code: 404,
                    ..
                }
            ) {
                warn!(task_id = %task_id, error = %e, "Failed to remove container");
            }
        }

        Ok(())
    }

    async fn container_statuses(
        &self,
        _task_id: &str,
    ) -> Result<Vec<ContainerObservedStatus>, SandboxError> {
        // Docker mode is agent-only — no server container management
        Ok(vec![])
    }

    async fn server_logs(
        &self,
        _task_id: &str,
        _server_name: &str,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError> {
        Err(SandboxError::NotReady(
            "Server logs not available in Docker mode".to_string(),
        ))
    }

    async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError> {
        self.require_sandbox_ready(task_id).await?;
        let container_name = generate_container_name(task_id);
        let url = format!(
            "http://{}:{}",
            container_name, self.config.sandbox.grpc_port
        );
        SandboxClient::connect(&url)
            .map_err(|e| SandboxError::Internal(format!("Failed to create gRPC client: {e}")))
    }
}

// =============================================================================
// Private helper methods
// =============================================================================

/// Result of checking for an existing container.
enum ExistingContainerResult {
    /// Container is running.
    Running,
    /// Container exists but is not running (created, paused, exited, etc.)
    Exists,
    /// No container found (404).
    NotFound,
}

impl DockerSandboxProvider {
    /// Fast readiness gate — checks container state before attempting gRPC.
    async fn require_sandbox_ready(&self, task_id: &str) -> Result<(), SandboxError> {
        let container_name = generate_container_name(task_id);
        match self
            .client
            .docker
            .inspect_container(&container_name, None)
            .await
        {
            Ok(info) => {
                let status_str = info
                    .state
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();

                if status_str == "running" {
                    Ok(())
                } else {
                    Err(SandboxError::NotReady(format!(
                        "Sandbox container is {status_str}"
                    )))
                }
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                ..
            }) => Err(SandboxError::NotReady(
                "Sandbox not provisioned".into(),
            )),
            Err(e) => Err(SandboxError::DockerError(format!(
                "Failed to inspect container: {e}"
            ))),
        }
    }

    /// Publish an "initializing" event (fire-and-forget).
    async fn publish_initializing(&self, task_id: &str) {
        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_sandbox_event(
                    task_id,
                    moatless_events::SandboxSubType::Initializing,
                    "Starting",
                )
                .await;
        }
    }

    /// Publish a "ready" event (fire-and-forget).
    async fn publish_ready(&self, task_id: &str) {
        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_sandbox_event(
                    task_id,
                    moatless_events::SandboxSubType::Ready,
                    "Running",
                )
                .await;
        }
    }

    /// Check if an existing container exists and its state.
    async fn check_existing_container(
        &self,
        container_name: &str,
    ) -> ExistingContainerResult {
        match self
            .client
            .docker
            .inspect_container(container_name, None)
            .await
        {
            Ok(info) => {
                let status_str = info
                    .state
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();

                if status_str == "running" {
                    ExistingContainerResult::Running
                } else {
                    ExistingContainerResult::Exists
                }
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                ..
            }) => ExistingContainerResult::NotFound,
            Err(e) => {
                warn!(container = %container_name, error = %e, "Error inspecting container");
                ExistingContainerResult::NotFound
            }
        }
    }
}
