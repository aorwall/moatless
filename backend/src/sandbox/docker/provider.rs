//! Docker implementation of the SandboxProvider trait.
//!
//! Manages sandbox lifecycle through Docker containers running as siblings
//! on a shared Docker network. Communication with sandbox containers uses
//! gRPC, resolved by container name as hostname on the Docker network.

use async_trait::async_trait;
use axum::response::sse::Event;
use bollard::container::{
    ListContainersOptions, RemoveContainerOptions, RestartContainerOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::network::ConnectNetworkOptions;
use bollard::models::EndpointSettings;
use futures::stream::BoxStream;
use std::sync::Arc;
use tracing::{info, warn};

use super::client::DockerClient;
use super::container_builder;
use super::naming::{generate_container_name, generate_server_container_name};
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
                    status: SandboxStatus::Ready,
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
                    status: SandboxStatus::Ready,
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

        // 3. Create workspace and data directories on host
        let workspace_path =
            super::naming::generate_workspace_host_path(&self.config.docker.data_dir, task_id);
        // Use container_data_dir for creating directories when running inside a container
        let container_workspace_path = super::naming::generate_workspace_host_path(
            &self.config.docker.container_data_dir,
            task_id,
        );
        let container_data_path = super::naming::generate_data_host_path(
            &self.config.docker.container_data_dir,
            task_id,
        );
        let container_claude_config_path = super::naming::generate_claude_config_host_path(
            &self.config.docker.container_data_dir,
            task_id,
        );
        for dir in [&container_workspace_path, &container_data_path, &container_claude_config_path] {
            if let Err(e) = tokio::fs::create_dir_all(dir).await {
                warn!(
                    task_id = %task_id,
                    path = %dir,
                    error = %e,
                    "Failed to create directory (may already exist)"
                );
            }
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

        // 6. Connect to extra Docker networks (opt-in)
        let extra_networks = plan
            .repo_config
            .docker_networks
            .as_deref()
            .unwrap_or(&self.config.docker.extra_networks);
        for network in extra_networks {
            if let Err(e) = self
                .client
                .docker
                .connect_network(
                    network,
                    ConnectNetworkOptions {
                        container: container_name.clone(),
                        endpoint_config: EndpointSettings::default(),
                    },
                )
                .await
            {
                warn!(
                    task_id = %task_id,
                    network = %network,
                    error = %e,
                    "Failed to connect sandbox to extra network (non-fatal)"
                );
            } else {
                info!(task_id = %task_id, network = %network, "Connected sandbox to extra network");
            }
        }

        // 7. Create and start server sidecar containers
        if let Some(ref servers) = plan.repo_config.servers {
            for server in servers {
                if let Err(e) = self.create_and_start_server_container(task_id, server, plan.repo_config.docker_image.as_deref()).await {
                    warn!(
                        task_id = %task_id,
                        server = %server.name,
                        error = %e,
                        "Failed to create server container (non-fatal)"
                    );
                }
            }
        }

        self.publish_ready(task_id).await;

        Ok(ProvisionResponse {
            status: SandboxStatus::Ready,
        })
    }

    async fn sandbox_status(
        &self,
        task_id: &str,
        server_configs: &[ServerConfig],
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        Ok(self.status_resolver.resolve_sandbox_status(task_id, server_configs).await)
    }

    async fn stop(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Stopping Docker sandbox");

        // Stop server containers first
        self.stop_server_containers(task_id).await;

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
                    moatless_events::SandboxStatus::Stopped,
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

        // Restart server containers
        self.restart_server_containers(task_id).await;

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

    async fn redeploy(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError> {
        info!(task_id = %task_id, "Redeploying Docker sandbox (remove containers, reprovision)");

        // Remove server containers
        self.remove_server_containers(task_id).await;

        // Remove sandbox container
        let container_name = generate_container_name(task_id);
        self.force_remove_container(&container_name, task_id).await;

        // Reprovision with fresh config (workspace dirs on host are preserved)
        self.provision(task_id, request).await
    }

    async fn cleanup(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Cleaning up Docker sandbox");

        // Remove server containers first
        self.remove_server_containers(task_id).await;

        let container_name = generate_container_name(task_id);

        // Force remove sandbox container (stops it if running)
        self.force_remove_container(&container_name, task_id).await;

        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_sandbox_event(
                    task_id,
                    moatless_events::SandboxStatus::Removed,
                    "Container removed",
                )
                .await;
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
        _previous: bool,
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
                    moatless_events::SandboxStatus::Initializing,
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
                    moatless_events::SandboxStatus::Ready,
                    "Running",
                )
                .await;
        }
    }

    /// Create and start a server sidecar container.
    async fn create_and_start_server_container(
        &self,
        task_id: &str,
        server: &ServerConfig,
        sandbox_image: Option<&str>,
    ) -> Result<(), SandboxError> {
        let server_container_name = generate_server_container_name(task_id, &server.name);

        // Check if it already exists
        match self.check_existing_container(&server_container_name).await {
            ExistingContainerResult::Running => {
                info!(task_id = %task_id, server = %server.name, "Reusing existing server container");
                return Ok(());
            }
            ExistingContainerResult::Exists => {
                info!(task_id = %task_id, server = %server.name, "Starting existing server container");
                self.client
                    .docker
                    .start_container(&server_container_name, None::<StartContainerOptions<String>>)
                    .await
                    .map_err(|e| SandboxError::DockerError(format!("Failed to start server container: {e}")))?;
                return Ok(());
            }
            ExistingContainerResult::NotFound => {}
        }

        let server_config = container_builder::build_server_container_config(
            &self.config,
            task_id,
            server,
            sandbox_image,
        );

        self.client
            .docker
            .create_container(
                Some(bollard::container::CreateContainerOptions {
                    name: &server_container_name,
                    platform: None,
                }),
                server_config,
            )
            .await
            .map_err(|e| SandboxError::DockerError(format!("Failed to create server container: {e}")))?;

        self.client
            .docker
            .start_container(&server_container_name, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| SandboxError::DockerError(format!("Failed to start server container: {e}")))?;

        info!(
            task_id = %task_id,
            server = %server.name,
            container = %server_container_name,
            "Server container created and started"
        );

        Ok(())
    }

    /// List all server container names for a task by label.
    async fn list_server_container_names(&self, task_id: &str) -> Vec<String> {
        use std::collections::HashMap;

        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![
            format!("moatless.io/task-id={task_id}"),
            "moatless.service=server".to_string(),
        ]);

        match self.client.docker.list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        })).await {
            Ok(containers) => containers
                .into_iter()
                .filter_map(|c| {
                    c.names
                        .and_then(|names| names.into_iter().next())
                        .map(|name| name.trim_start_matches('/').to_string())
                })
                .collect(),
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to list server containers");
                vec![]
            }
        }
    }

    /// Stop all server containers for a task.
    async fn stop_server_containers(&self, task_id: &str) {
        for name in self.list_server_container_names(task_id).await {
            if let Err(e) = self.client.docker.stop_container(
                &name,
                Some(StopContainerOptions { t: 10 }),
            ).await {
                if !matches!(e, bollard::errors::Error::DockerResponseServerError { status_code: 304 | 404, .. }) {
                    warn!(task_id = %task_id, container = %name, error = %e, "Failed to stop server container");
                }
            }
        }
    }

    /// Restart all server containers for a task.
    async fn restart_server_containers(&self, task_id: &str) {
        for name in self.list_server_container_names(task_id).await {
            if let Err(e) = self.client.docker.restart_container(
                &name,
                Some(RestartContainerOptions { t: 10 }),
            ).await {
                warn!(task_id = %task_id, container = %name, error = %e, "Failed to restart server container");
            }
        }
    }

    /// Force-remove all server containers for a task.
    async fn remove_server_containers(&self, task_id: &str) {
        for name in self.list_server_container_names(task_id).await {
            self.force_remove_container(&name, task_id).await;
        }
    }

    /// Force-remove a container, ignoring 404 (already gone).
    async fn force_remove_container(&self, container_name: &str, task_id: &str) {
        if let Err(e) = self
            .client
            .docker
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await
        {
            if !matches!(
                e,
                bollard::errors::Error::DockerResponseServerError {
                    status_code: 404,
                    ..
                }
            ) {
                warn!(task_id = %task_id, container = %container_name, error = %e, "Failed to remove container");
            }
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
