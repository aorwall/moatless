//! Docker implementation of the SandboxProvider trait.
//!
//! Manages sandbox lifecycle through Docker containers running as siblings
//! on a shared Docker network. Communication with sandbox containers uses
//! gRPC, resolved by container name as hostname on the Docker network.

use async_trait::async_trait;
use axum::response::sse::Event;
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, EndpointSettings, NetworkConnectRequest,
    NetworkingConfig,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, ListContainersOptionsBuilder, RemoveContainerOptionsBuilder,
    RestartContainerOptionsBuilder, StartContainerOptions, StopContainerOptionsBuilder,
};
use futures::stream::BoxStream;
use std::sync::Arc;
use tracing::{error, info, warn};

use super::client::DockerClient;
use super::container_builder;
use super::naming::{generate_container_name, generate_server_container_name};
use super::status_resolver::DockerStatusResolver;
use crate::config::Config;
use crate::events::EventPublisher;
use crate::sandbox::client::SandboxClient;
use crate::sandbox::error::SandboxError;
use crate::sandbox::jwt;
use crate::sandbox::lifecycle::RuntimeOperationContext;
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
        ctx: RuntimeOperationContext,
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
            ExistingContainerResult::Exists { current_image } => {
                let expected = container_builder::resolve_sandbox_image(&self.config, plan);
                if current_image.as_deref() == Some(expected.as_str()) {
                    // Tag unchanged — fast path: start the existing container.
                    info!(task_id = %task_id, image = %expected, "Starting existing stopped container (image unchanged)");
                    self.client
                        .docker
                        .start_container(&container_name, None::<StartContainerOptions>)
                        .await
                        .map_err(|e| {
                            SandboxError::DockerError(format!("Failed to start container: {e}"))
                        })?;

                    return Ok(ProvisionResponse {
                        status: SandboxStatus::Ready,
                    });
                }

                // Tag changed — remove the stale container (and its servers) and
                // fall through to recreate from the fresh plan with the new image.
                info!(
                    task_id = %task_id,
                    old_image = ?current_image,
                    new_image = %expected,
                    "Sandbox image changed since stop, recreating container"
                );
                self.force_remove_container(&container_name, task_id).await?;
                self.remove_server_containers(task_id).await;
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

        // Docker mode is not etcd-exposed, so the long-lived token stays on the
        // container env (USER_AUTH_TOKEN) as before — only the K8s path moves it
        // to the credential channel.
        let user_auth_token = jwt::generate_user_auth_token(
            &request.user_id,
            &request.user_id,
            &task_token_secret,
            self.config.sandbox.user_token_ttl_secs,
        )
        .map_err(|e| SandboxError::JwtError(e.to_string()))?;

        // 3. Create workspace and data directories on host
        let workspace_path =
            super::naming::generate_sandbox_host_path(&self.config.docker.data_dir, task_id);
        // Use container_data_dir for creating directories when running inside a container
        let container_workspace_path = super::naming::generate_sandbox_host_path(
            &self.config.docker.container_data_dir,
            task_id,
        );
        let container_data_path =
            super::naming::generate_data_host_path(&self.config.docker.container_data_dir, task_id);
        let container_claude_config_path = super::naming::generate_claude_config_host_path(
            &self.config.docker.container_data_dir,
            task_id,
        );
        for dir in [
            &container_workspace_path,
            &container_data_path,
            &container_claude_config_path,
        ] {
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
        let mut container_config = container_builder::build_container_config(
            &self.config,
            task_id,
            plan,
            Some(&user_auth_token),
        );
        container_builder::stamp_operation_generation(&mut container_config, ctx);

        // 5. Create and start container
        self.client
            .docker
            .create_container(
                Some(
                    CreateContainerOptionsBuilder::new()
                        .name(&container_name)
                        .build(),
                ),
                container_config,
            )
            .await
            .map_err(|e| SandboxError::DockerError(format!("Failed to create container: {e}")))?;

        self.client
            .docker
            .start_container(&container_name, None::<StartContainerOptions>)
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
                    NetworkConnectRequest {
                        container: container_name.clone(),
                        endpoint_config: Some(EndpointSettings::default()),
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

        // 7. Create and start server sidecar containers. Secrets are
        //    materialized to `/opt/moatless/data/server.env` by the sandbox
        //    init after it fetches them via gRPC `GetCredentials`.
        if let Some(ref servers) = plan.repo_config.servers {
            // Validate all server configs up front so we fail fast before
            // kicking off any container create calls. Publishes a `failed`
            // event per broken server so operators see the reason in
            // `moat tasks diagnose` / the event feed.
            let sandbox_image: &str = plan
                .repo_config
                .docker_image
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(&self.config.docker.image);
            for server in servers {
                if let Err(msg) = container_builder::validate_server_config(server) {
                    error!(
                        task_id = %task_id,
                        server = %server.name,
                        "Invalid server config: {msg}"
                    );
                    self.publish_server_failed(task_id, &server.name, &msg)
                        .await;
                    return Err(SandboxError::DockerError(msg));
                }
            }

            for server in servers {
                if let Err(e) = self
                    .create_and_start_server_container(task_id, server, sandbox_image)
                    .await
                {
                    let msg = format!("Failed to create server container: {e}");
                    error!(
                        task_id = %task_id,
                        server = %server.name,
                        error = %e,
                        "Failed to create server container"
                    );
                    self.publish_server_failed(task_id, &server.name, &msg)
                        .await;
                    return Err(e);
                }
            }
        }

        Ok(ProvisionResponse {
            status: SandboxStatus::Ready,
        })
    }

    async fn sandbox_status(
        &self,
        task_id: &str,
        server_configs: &[ServerConfig],
        task_status: crate::task::models::TaskStatus,
        desired_state: crate::task::models::SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        Ok(self
            .status_resolver
            .resolve_sandbox_status(task_id, server_configs, task_status, desired_state)
            .await)
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: crate::task::models::TaskStatus,
        desired_state: crate::task::models::SandboxDesiredState,
    ) -> Result<crate::sandbox::live_status::LiveStatus, SandboxError> {
        Ok(self
            .status_resolver
            .resolve_live_status(task_id, task_status, desired_state)
            .await)
    }

    fn expected_sandbox_image(&self, repo_docker_image: Option<&str>) -> String {
        repo_docker_image
            .filter(|s| !s.is_empty())
            .map_or_else(|| self.config.docker.image.clone(), str::to_string)
    }

    async fn stop(
        &self,
        task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Stopping Docker sandbox");

        // Stop server containers first
        self.stop_server_containers(task_id).await;

        let container_name = generate_container_name(task_id);

        self.client
            .docker
            .stop_container(
                &container_name,
                Some(StopContainerOptionsBuilder::new().t(30).build()),
            )
            .await
            .map_err(|e| SandboxError::DockerError(format!("Failed to stop container: {e}")))?;

        Ok(())
    }

    async fn restart(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Recreating Docker sandbox for restart");

        let container_name = generate_container_name(task_id);
        let inspected = self
            .client
            .docker
            .inspect_container(&container_name, None)
            .await
            .map_err(|e| {
                SandboxError::DockerError(format!(
                    "Failed to inspect container before restart: {e}"
                ))
            })?;
        let container_config = recreate_config(inspected, ctx)?;

        // Labels are immutable on a live Docker container. Strictly remove the
        // sandbox and recreate it from the inspected configuration so the
        // replacement has a new container ID and generation label.
        self.force_remove_container(&container_name, task_id).await?;
        self.client
            .docker
            .create_container(
                Some(
                    CreateContainerOptionsBuilder::new()
                        .name(&container_name)
                        .build(),
                ),
                container_config,
            )
            .await
            .map_err(|e| {
                SandboxError::DockerError(format!(
                    "Failed to recreate container during restart: {e}"
                ))
            })?;
        self.client
            .docker
            .start_container(&container_name, None::<StartContainerOptions>)
            .await
            .map_err(|e| {
                SandboxError::DockerError(format!(
                    "Failed to start replacement container during restart: {e}"
                ))
            })?;

        // Server containers are subordinate resources; their lifecycle events
        // are excluded from sandbox observation.
        self.restart_server_containers(task_id).await;

        Ok(())
    }

    async fn redeploy(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
        ctx: RuntimeOperationContext,
    ) -> Result<ProvisionResponse, SandboxError> {
        info!(task_id = %task_id, "Redeploying Docker sandbox (remove containers, reprovision)");

        // Remove server containers (subordinate — best-effort).
        self.remove_server_containers(task_id).await;

        // Strict edge teardown: never provision past a failed remove, or the
        // fresh plan would reuse the stale sandbox container.
        let container_name = generate_container_name(task_id);
        self.force_remove_container(&container_name, task_id).await?;

        // Reprovision with fresh config (workspace dirs on host are preserved)
        self.provision(task_id, request, ctx).await
    }

    async fn cleanup(
        &self,
        task_id: &str,
        _retain_storage: bool,
        _ctx: RuntimeOperationContext,
    ) -> Result<(), SandboxError> {
        // Docker mode has no S3-backed storage sync, so `retain_storage` is a
        // no-op here — there is nothing to preserve or delete beyond the
        // containers themselves.
        info!(task_id = %task_id, "Cleaning up Docker sandbox");

        // Remove server containers first (subordinate — best-effort).
        self.remove_server_containers(task_id).await;

        let container_name = generate_container_name(task_id);

        // Load-bearing: strict removal must not report success while the sandbox
        // container lingers — a propagated error withholds lifecycle's `Removed`
        // completion, the sole publisher of that fact.
        self.force_remove_container(&container_name, task_id).await?;

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
    /// Container exists but is not running (created, paused, exited, etc.).
    /// Carries the image the container was created with so the start path can
    /// decide whether to start it (tag unchanged) or recreate it (tag changed).
    Exists { current_image: Option<String> },
    /// No container found (404).
    NotFound,
}

/// Reconstruct the create request for a Docker restart from the current
/// container's effective configuration, replacing only runtime-assigned
/// network details and lifecycle's immutable generation label.
fn recreate_config(
    inspected: ContainerInspectResponse,
    ctx: RuntimeOperationContext,
) -> Result<ContainerCreateBody, SandboxError> {
    let config = inspected.config.ok_or_else(|| {
        SandboxError::DockerError("Container inspect response had no config".to_string())
    })?;
    let value = serde_json::to_value(config).map_err(|e| {
        SandboxError::DockerError(format!("Failed to serialize inspected container config: {e}"))
    })?;
    let mut body: ContainerCreateBody = serde_json::from_value(value).map_err(|e| {
        SandboxError::DockerError(format!("Failed to rebuild inspected container config: {e}"))
    })?;
    body.host_config = inspected.host_config;
    body.networking_config = inspected
        .network_settings
        .and_then(|settings| settings.networks)
        .map(|networks| NetworkingConfig {
            endpoints_config: Some(
                networks
                    .into_keys()
                    .map(|name| (name, EndpointSettings::default()))
                    .collect(),
            ),
        });
    container_builder::stamp_operation_generation(&mut body, ctx);
    Ok(body)
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
                status_code: 404, ..
            }) => Err(SandboxError::NotReady("Sandbox not provisioned".into())),
            Err(e) => Err(SandboxError::DockerError(format!(
                "Failed to inspect container: {e}"
            ))),
        }
    }

    /// Publish a server `failed` event with the operator-facing reason.
    ///
    /// Surfaces in `moat tasks diagnose` (via the `isError: true` filter on
    /// the event feed) so crashes during provisioning don't require digging
    /// through backend logs.
    async fn publish_server_failed(&self, task_id: &str, server_name: &str, message: &str) {
        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_server_event(
                    task_id,
                    moatless_events::ServerSubType::Failed,
                    &moatless_events::ServerEventPayload {
                        server_name: server_name.to_string(),
                        status: moatless_events::ServerSubType::Failed,
                        message: message.to_string(),
                    },
                )
                .await;
        }
    }

    /// Create and start a server sidecar container.
    async fn create_and_start_server_container(
        &self,
        task_id: &str,
        server: &ServerConfig,
        sandbox_image: &str,
    ) -> Result<(), SandboxError> {
        let server_container_name = generate_server_container_name(task_id, &server.name);

        // Check if it already exists
        match self.check_existing_container(&server_container_name).await {
            ExistingContainerResult::Running => {
                info!(task_id = %task_id, server = %server.name, "Reusing existing server container");
                return Ok(());
            }
            ExistingContainerResult::Exists { .. } => {
                info!(task_id = %task_id, server = %server.name, "Starting existing server container");
                self.client
                    .docker
                    .start_container(&server_container_name, None::<StartContainerOptions>)
                    .await
                    .map_err(|e| {
                        SandboxError::DockerError(format!("Failed to start server container: {e}"))
                    })?;
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
                Some(
                    CreateContainerOptionsBuilder::new()
                        .name(&server_container_name)
                        .build(),
                ),
                server_config,
            )
            .await
            .map_err(|e| {
                SandboxError::DockerError(format!("Failed to create server container: {e}"))
            })?;

        self.client
            .docker
            .start_container(&server_container_name, None::<StartContainerOptions>)
            .await
            .map_err(|e| {
                SandboxError::DockerError(format!("Failed to start server container: {e}"))
            })?;

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
        filters.insert(
            "label".to_string(),
            vec![
                format!("moatless.io/task-id={task_id}"),
                "moatless.service=server".to_string(),
            ],
        );

        match self
            .client
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::new()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
        {
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
            if let Err(e) = self
                .client
                .docker
                .stop_container(
                    &name,
                    Some(StopContainerOptionsBuilder::new().t(10).build()),
                )
                .await
            {
                if !matches!(
                    e,
                    bollard::errors::Error::DockerResponseServerError {
                        status_code: 304 | 404,
                        ..
                    }
                ) {
                    warn!(task_id = %task_id, container = %name, error = %e, "Failed to stop server container");
                }
            }
        }
    }

    /// Restart all server containers for a task.
    async fn restart_server_containers(&self, task_id: &str) {
        for name in self.list_server_container_names(task_id).await {
            if let Err(e) = self
                .client
                .docker
                .restart_container(
                    &name,
                    Some(RestartContainerOptionsBuilder::new().t(10).build()),
                )
                .await
            {
                warn!(task_id = %task_id, container = %name, error = %e, "Failed to restart server container");
            }
        }
    }

    /// Force-remove all server containers for a task.
    /// Server containers are subordinate runtime resources — best-effort.
    async fn remove_server_containers(&self, task_id: &str) {
        for name in self.list_server_container_names(task_id).await {
            if let Err(e) = self.force_remove_container(&name, task_id).await {
                warn!(
                    task_id,
                    container = %name,
                    error = %e,
                    "Failed to remove subordinate server container"
                );
            }
        }
    }

    /// Force-remove a container. `Ok` on success or when it is already gone
    /// (404); a non-404 error propagates so removal can be load-bearing —
    /// strict cleanup and edge teardown must not proceed past a failed remove.
    async fn force_remove_container(
        &self,
        container_name: &str,
        task_id: &str,
    ) -> Result<(), SandboxError> {
        match self
            .client
            .docker
            .remove_container(
                container_name,
                Some(
                    RemoveContainerOptionsBuilder::new()
                        .force(true)
                        .v(true)
                        .build(),
                ),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                ..
            }) => Ok(()),
            Err(e) => {
                warn!(task_id = %task_id, container = %container_name, error = %e, "Failed to remove container");
                Err(SandboxError::DockerError(format!(
                    "remove container {container_name}: {e}"
                )))
            }
        }
    }

    /// Check if an existing container exists and its state.
    async fn check_existing_container(&self, container_name: &str) -> ExistingContainerResult {
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
                    let current_image = info.config.and_then(|c| c.image);
                    ExistingContainerResult::Exists { current_image }
                }
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => ExistingContainerResult::NotFound,
            Err(e) => {
                warn!(container = %container_name, error = %e, "Error inspecting container");
                ExistingContainerResult::NotFound
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::lifecycle::runtime::OPERATION_GENERATION_LABEL;
    use crate::sandbox::lifecycle::OperationGeneration;
    use std::collections::HashMap;

    #[test]
    fn restart_rebuilds_config_and_replaces_generation_label() {
        let inspected = ContainerInspectResponse {
            config: Some(bollard::models::ContainerConfig {
                image: Some("sandbox:v1".to_string()),
                env: Some(vec!["TASK_ID=t1".to_string()]),
                labels: Some(HashMap::from([(
                    OPERATION_GENERATION_LABEL.to_string(),
                    "4".to_string(),
                )])),
                ..Default::default()
            }),
            host_config: Some(bollard::models::HostConfig {
                network_mode: Some("moatless".to_string()),
                ..Default::default()
            }),
            network_settings: Some(bollard::models::NetworkSettings {
                networks: Some(HashMap::from([(
                    "preview".to_string(),
                    EndpointSettings::default(),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        };

        let rebuilt = recreate_config(
            inspected,
            RuntimeOperationContext::new(OperationGeneration(5)),
        )
        .expect("recreate config");

        assert_eq!(rebuilt.image.as_deref(), Some("sandbox:v1"));
        assert_eq!(rebuilt.env.as_deref(), Some(&["TASK_ID=t1".to_string()][..]));
        assert_eq!(
            rebuilt
                .labels
                .as_ref()
                .and_then(|labels| labels.get(OPERATION_GENERATION_LABEL))
                .map(String::as_str),
            Some("5")
        );
        assert_eq!(
            rebuilt
                .host_config
                .as_ref()
                .and_then(|host| host.network_mode.as_deref()),
            Some("moatless")
        );
        assert!(rebuilt
            .networking_config
            .and_then(|networking| networking.endpoints_config)
            .is_some_and(|networks| networks.contains_key("preview")));
    }
}
