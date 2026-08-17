//! Docker Sandbox runtime, observer, and connector adapter.
//!
//! Manages sandbox lifecycle through Docker containers running as siblings
//! on a shared Docker network. Communication with sandbox containers uses
//! gRPC, resolved by container name as hostname on the Docker network.

use async_trait::async_trait;
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
use tracing::{debug, error, info, warn};

use super::client::DockerClient;
use super::container_builder;
use super::naming::{
    generate_container_name, generate_server_container_name, generate_task_host_path,
};
use super::status_resolver::DockerStatusResolver;
use moatless_backend_core::sandbox as core_sandbox;
use moatless_core::config::Config;
use moatless_sandbox_control::client::SandboxClient;
use moatless_sandbox_control::error::SandboxError;
use moatless_sandbox_control::jwt;
use moatless_sandbox_control::lifecycle::outcome::{
    DegradationWarning, Diagnostics, ServerFailure,
};
use moatless_sandbox_control::lifecycle::runtime::{
    RuntimeOperationContext, RuntimeOutcome, SandboxRuntime, map_sandbox_error,
};
use moatless_sandbox_control::lifecycle::{
    ArchiveRetention, ObservedStatus, ProvisionDeps, RuntimeEffect,
};
use moatless_sandbox_control::models::*;
use moatless_sandbox_control::schemas::*;
use moatless_sandbox_control::status_reader::SandboxStatusReader;
use moatless_workspaces::repository::models::ServerConfig;

/// Docker adapter for lifecycle mutation, observation, and connection.
pub struct DockerSandboxAdapter {
    pub config: Arc<Config>,
    pub client: DockerClient,
    pub status_resolver: DockerStatusResolver,
    // `pub` (not `pub(crate)`) is the carve's declared debt: the backend
    // composition root (`startup.rs::build_docker_sandbox_adapter`) builds this
    // adapter with a struct literal across the crate boundary, and a cross-crate
    // struct literal requires every field it sets to be `pub`. Mirrors
    // `K8sSandboxAdapter.plan`.
    pub plan: Option<ProvisionDeps>,
    pub s3: Option<Arc<aws_sdk_s3::Client>>,
}

async fn apply_disk_archive_retention(
    container_data_dir: &str,
    task_id: &str,
    retention: ArchiveRetention,
) -> Result<(), moatless_sandbox_control::lifecycle::RuntimeError> {
    if retention.retains() {
        return Ok(());
    }

    let task_path = generate_task_host_path(container_data_dir, task_id)
        .map_err(moatless_sandbox_control::lifecycle::RuntimeError::Rejected)?;
    match tokio::fs::remove_dir_all(&task_path).await {
        Ok(()) => {
            info!(task_id, path = %task_path.display(), "Deleted Docker task disk archives");
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(
            moatless_sandbox_control::lifecycle::RuntimeError::Infrastructure(format!(
                "delete Docker task disk archives at {}: {error}",
                task_path.display()
            )),
        ),
    }
}

impl DockerSandboxAdapter {
    fn require_plan(
        &self,
    ) -> Result<&ProvisionDeps, moatless_sandbox_control::lifecycle::RuntimeError> {
        self.plan.as_ref().ok_or_else(|| {
            moatless_sandbox_control::lifecycle::RuntimeError::Infrastructure(
                "sandbox plan repositories unavailable".to_string(),
            )
        })
    }

    async fn ensure_running_with_request(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, SandboxError> {
        info!(task_id = %task_id, user_id = %request.user_id, "Provisioning Docker sandbox");

        let container_name = generate_container_name(task_id);
        let plan = &request.plan;
        let mut diagnostics = Diagnostics::default();

        // 1. Check for existing container — idempotent, never force-remove
        match self.check_existing_container(&container_name).await {
            ExistingContainerResult::Running => {
                info!(task_id = %task_id, "Reusing existing running container");
                return Ok(RuntimeOutcome {
                    observed: ObservedStatus::Ready,
                    effect: RuntimeEffect::NoOp,
                    diagnostics,
                });
            }
            ExistingContainerResult::Exists { current_image } => {
                let expected = container_builder::resolve_sandbox_image(&self.config, plan);
                if current_image.as_deref() == Some(expected.as_str()) {
                    // Docker labels are immutable, so a new Start generation
                    // must replace even an image-compatible stopped container.
                    info!(task_id = %task_id, image = %expected, "Recreating stopped container for the current generation");
                    let inspected = self
                        .client
                        .docker
                        .inspect_container(&container_name, None)
                        .await
                        .map_err(|source| SandboxError::Docker {
                            operation: "inspecting a stopped container before recreation",
                            source: Box::new(source),
                        })?;
                    let container_config = recreate_config(inspected, ctx)?;
                    self.force_remove_container(&container_name).await?;
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
                        .map_err(|source| SandboxError::Docker {
                            operation: "recreating a stopped container",
                            source: Box::new(source),
                        })?;
                    self.client
                        .docker
                        .start_container(&container_name, None::<StartContainerOptions>)
                        .await
                        .map_err(|source| SandboxError::Docker {
                            operation: "starting a recreated container",
                            source: Box::new(source),
                        })?;

                    return Ok(RuntimeOutcome {
                        observed: ObservedStatus::Ready,
                        effect: RuntimeEffect::Applied,
                        diagnostics,
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
                self.force_remove_container(&container_name).await?;
                self.ensure_container_absent(&container_name, task_id)
                    .await?;
                self.remove_server_containers(task_id).await?;
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
        // container env (USER_AUTH_TOKEN).
        let user_auth_token = jwt::generate_user_auth_token(
            &request.user_id,
            &request.user_id,
            task_id,
            &task_token_secret,
            self.config.sandbox.user_token_ttl_secs,
        )
        .map_err(|source| SandboxError::JwtSigning {
            purpose: "the sandbox user auth token",
            source,
        })?;

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
                diagnostics.degradations.push(DegradationWarning {
                    code: "host_directory_unavailable".to_string(),
                    message: format!("Failed to prepare {dir}: {e}"),
                });
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
            .map_err(|source| SandboxError::Docker {
                operation: "creating the sandbox container",
                source: Box::new(source),
            })?;

        self.client
            .docker
            .start_container(&container_name, None::<StartContainerOptions>)
            .await
            .map_err(|source| SandboxError::Docker {
                operation: "starting the sandbox container",
                source: Box::new(source),
            })?;

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
                diagnostics.degradations.push(DegradationWarning {
                    code: "network_degraded".to_string(),
                    message: format!("Failed to connect network {network}: {e}"),
                });
            } else {
                info!(task_id = %task_id, network = %network, "Connected sandbox to extra network");
            }
        }

        // 7. Create and start server sidecar containers. Secrets are
        //    materialized to `/opt/moatless/data/server.env` by the sandbox
        //    init after it fetches them via gRPC `GetCredentials`.
        if let Some(ref servers) = plan.repo_config.servers {
            let sandbox_image: &str = plan
                .repo_config
                .docker_image
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(&self.config.docker.image);
            for server in servers {
                if let Err(message) = container_builder::validate_server_config(server) {
                    error!(
                        task_id = %task_id,
                        server = %server.name,
                        "Invalid server config: {message}"
                    );
                    diagnostics.server_failures.push(ServerFailure {
                        server: server.name.clone(),
                        message,
                    });
                    continue;
                }
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
                    diagnostics.server_failures.push(ServerFailure {
                        server: server.name.clone(),
                        message: msg,
                    });
                }
            }
        }

        Ok(RuntimeOutcome {
            observed: ObservedStatus::Ready,
            effect: RuntimeEffect::Applied,
            diagnostics,
        })
    }
}

#[async_trait]
impl SandboxRuntime for DockerSandboxAdapter {
    async fn ensure_running(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        let plan = self.require_plan()?;
        let request = plan.build_request(task_id).await?;
        self.ensure_running_with_request(task_id, &request, ctx)
            .await
            .map_err(map_sandbox_error)
    }

    async fn stop(
        &self,
        task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Stopping Docker sandbox");

        // Stop server containers first
        self.stop_server_containers(task_id).await;

        let container_name = generate_container_name(task_id);
        let effect = match self
            .client
            .docker
            .inspect_container(&container_name, None)
            .await
        {
            Ok(inspected)
                if inspected
                    .state
                    .as_ref()
                    .and_then(|state| state.running)
                    .unwrap_or(false) =>
            {
                self.client
                    .docker
                    .stop_container(
                        &container_name,
                        Some(StopContainerOptionsBuilder::new().t(30).build()),
                    )
                    .await
                    .map_err(|source| {
                        map_sandbox_error(SandboxError::Docker {
                            operation: "stopping the sandbox container",
                            source: Box::new(source),
                        })
                    })?;
                RuntimeEffect::Applied
            }
            Ok(_) => RuntimeEffect::NoOp,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => RuntimeEffect::NoOp,
            Err(source) => {
                return Err(map_sandbox_error(SandboxError::Docker {
                    operation: "inspecting the sandbox container before stop",
                    source: Box::new(source),
                }));
            }
        };

        Ok(RuntimeOutcome {
            observed: ObservedStatus::Stopped,
            effect,
            diagnostics: Diagnostics::default(),
        })
    }

    async fn restart(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Recreating Docker sandbox for restart");

        let container_name = generate_container_name(task_id);
        let inspected = self
            .client
            .docker
            .inspect_container(&container_name, None)
            .await
            .map_err(|source| {
                map_sandbox_error(SandboxError::Docker {
                    operation: "inspecting the sandbox container before restart",
                    source: Box::new(source),
                })
            })?;
        let container_config = recreate_config(inspected, ctx).map_err(map_sandbox_error)?;

        // Labels are immutable on a live Docker container. Strictly remove the
        // sandbox and recreate it from the inspected configuration so the
        // replacement has a new container ID and generation label.
        self.force_remove_container(&container_name)
            .await
            .map_err(map_sandbox_error)?;
        self.ensure_container_absent_runtime(&container_name, task_id)
            .await?;
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
            .map_err(|source| {
                map_sandbox_error(SandboxError::Docker {
                    operation: "recreating the sandbox container during restart",
                    source: Box::new(source),
                })
            })?;
        self.client
            .docker
            .start_container(&container_name, None::<StartContainerOptions>)
            .await
            .map_err(|source| {
                map_sandbox_error(SandboxError::Docker {
                    operation: "starting the replacement container during restart",
                    source: Box::new(source),
                })
            })?;

        // Server containers are subordinate resources; their lifecycle events
        // are excluded from sandbox observation.
        self.restart_server_containers(task_id).await;

        Ok(RuntimeOutcome::applied(ObservedStatus::Ready))
    }

    async fn redeploy(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Redeploying Docker sandbox (remove containers, reprovision)");
        let plan = self.require_plan()?;
        let request = plan.build_request(task_id).await?;

        // A redeploy must cross a clean edge: stale subordinate containers can
        // otherwise survive with obsolete server configuration.
        self.remove_server_containers(task_id)
            .await
            .map_err(map_sandbox_error)?;

        // Strict edge teardown: never provision past a failed remove, or the
        // fresh plan would reuse the stale sandbox container.
        let container_name = generate_container_name(task_id);
        self.force_remove_container(&container_name)
            .await
            .map_err(map_sandbox_error)?;
        self.ensure_container_absent_runtime(&container_name, task_id)
            .await?;

        // Reprovision with fresh config (workspace dirs on host are preserved)
        self.ensure_running_with_request(task_id, &request, ctx)
            .await
            .map_err(map_sandbox_error)
    }

    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Cleaning up Docker sandbox");
        let container_name = generate_container_name(task_id);
        let server_containers = self
            .list_server_container_names(task_id)
            .await
            .map_err(map_sandbox_error)?;
        let had_runtime = self
            .container_exists(&container_name)
            .await
            .map_err(map_sandbox_error)?
            || !server_containers.is_empty();

        self.remove_named_server_containers(task_id, server_containers)
            .await
            .map_err(map_sandbox_error)?;

        // Load-bearing: strict removal must not report success while the sandbox
        // container lingers — a propagated error withholds lifecycle's `Removed`
        // completion, the sole publisher of that fact.
        self.force_remove_container(&container_name)
            .await
            .map_err(map_sandbox_error)?;
        self.ensure_container_absent_runtime(&container_name, task_id)
            .await?;
        if !self
            .list_server_container_names(task_id)
            .await
            .map_err(map_sandbox_error)?
            .is_empty()
        {
            return Err(
                moatless_sandbox_control::lifecycle::RuntimeError::Infrastructure(format!(
                    "Docker resources for task {task_id} remain after removal"
                )),
            );
        }
        if !retention.retains() {
            apply_disk_archive_retention(
                &self.config.docker.container_data_dir,
                task_id,
                retention,
            )
            .await?;
            self.delete_archive_prefix(task_id, &self.config.s3.transcript_prefix, "transcripts")
                .await?;
            self.delete_archive_prefix(task_id, &self.config.s3.workspace_prefix, "workspace")
                .await?;
        }

        Ok(RuntimeOutcome {
            observed: ObservedStatus::Removed,
            effect: if had_runtime {
                RuntimeEffect::Applied
            } else {
                RuntimeEffect::NoOp
            },
            diagnostics: Diagnostics::default(),
        })
    }

    async fn reclaim_storage(
        &self,
        task_id: &str,
        _rescue_upload_url: Option<&str>,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        // The Docker runtime bind-mounts host paths and provisions no volume to
        // reclaim, so there is no storage tier to fall down. Reporting `NoOp`
        // rather than an error keeps a Docker deployment's reaper cycle quiet.
        debug!(task_id = %task_id, "Storage reclaim is a no-op on the Docker runtime");
        Ok(RuntimeOutcome {
            observed: ObservedStatus::Stopped,
            effect: RuntimeEffect::NoOp,
            diagnostics: Diagnostics::default(),
        })
    }
}

#[async_trait]
impl SandboxStatusReader for DockerSandboxAdapter {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: moatless_backend_core::models::TaskStatus,
        desired_state: moatless_backend_core::models::SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        let sandbox_id = self.sandbox_id_for_task(task_id).await?;
        let server_configs = if let Some(plan) = &self.plan {
            plan.server_configs(task_id).await
        } else {
            warn!(task_id, "Sandbox plan repositories unavailable for status");
            Vec::new()
        };
        Ok(self
            .status_resolver
            .resolve_sandbox_status(
                task_id,
                &sandbox_id,
                &server_configs,
                task_status,
                desired_state,
                self,
            )
            .await)
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: moatless_backend_core::models::TaskStatus,
        desired_state: moatless_backend_core::models::SandboxDesiredState,
    ) -> Result<moatless_sandbox_control::live_status::LiveStatus, SandboxError> {
        let sandbox_id = self.sandbox_id_for_task(task_id).await?;
        Ok(self
            .status_resolver
            .resolve_live_status(task_id, &sandbox_id, task_status, desired_state, self)
            .await)
    }

    async fn server_logs(
        &self,
        _task_id: &str,
        _server_name: &str,
        _options: moatless_sandbox_control::access::LogOptions,
    ) -> Result<BoxStream<'static, Result<String, SandboxError>>, SandboxError> {
        Err(SandboxError::NotReady(
            "Server logs not available in Docker mode".to_string(),
        ))
    }
}

#[async_trait]
impl core_sandbox::SandboxConnector for DockerSandboxAdapter {
    async fn sandbox_client(
        &self,
        task_id: &str,
    ) -> Result<SandboxClient, core_sandbox::SandboxError> {
        let sandbox_id = self
            .sandbox_id_for_task(task_id)
            .await
            .map_err(SandboxError::into_contract)?;
        self.require_sandbox_ready(&sandbox_id)
            .await
            .map_err(SandboxError::into_contract)?;
        let container_name = generate_container_name(&sandbox_id);
        let url = format!(
            "http://{}:{}",
            container_name, self.config.sandbox.grpc_port
        );
        SandboxClient::connect(&url)
            .map_err(|e| core_sandbox::SandboxError::GrpcClient(e.to_string()))
    }
}

impl DockerSandboxAdapter {
    async fn sandbox_id_for_task(&self, task_id: &str) -> Result<String, SandboxError> {
        match &self.plan {
            Some(plan) => plan
                .sandbox_id_for_task(task_id)
                .await
                .map_err(SandboxError::Runtime),
            None => Ok(task_id.to_string()),
        }
    }
}

/// Docker mode cannot publish a port after the fact: the agent container fixes
/// its published ports at creation (`container_builder.rs`) and publishes only
/// the sandbox HTTP and gRPC ports, so a port a process binds later has no way
/// out. `None` rather than an error — a Script still runs, it just gets no URL.
///
/// A reserved port is still refused, exactly as Kubernetes refuses it. Nothing
/// is externally routable here, so this is not the escape the Kubernetes check
/// prevents; it is the same rule giving the same answer in both runtimes, and a
/// script binding the control port would collide with the sandbox's own
/// listener inside the container regardless.
#[async_trait]
impl core_sandbox::SandboxPortPublisher for DockerSandboxAdapter {
    async fn publish_port(
        &self,
        task_id: &str,
        _name: &str,
        port: u16,
    ) -> Result<Option<String>, core_sandbox::SandboxError> {
        if core_sandbox::is_reserved_sandbox_port(port, self.config.sandbox.grpc_port) {
            return Err(core_sandbox::SandboxError::BadRequest(format!(
                "Port {port} is reserved by the sandbox and cannot be exposed"
            )));
        }

        debug!(
            task_id,
            port, "Docker mode does not publish sandbox ports; no preview URL"
        );
        Ok(None)
    }
}

/// Local Docker has no fleet-wide view — a container is only ever addressed by
/// its Task. Empty rather than an error, so the diagnostics page renders
/// normally under `docker-compose`.
#[async_trait]
impl core_sandbox::ClusterDiagnostics for DockerSandboxAdapter {
    async fn list_sandbox_diagnostics(
        &self,
    ) -> Result<Vec<core_sandbox::SandboxDiagnosticsEntry>, core_sandbox::SandboxError> {
        Ok(Vec::new())
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
    let value = serde_json::to_value(config).map_err(SandboxError::DockerConfig)?;
    let mut body: ContainerCreateBody =
        serde_json::from_value(value).map_err(SandboxError::DockerConfig)?;
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

impl DockerSandboxAdapter {
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
            Err(source) => Err(SandboxError::Docker {
                operation: "inspecting the sandbox container",
                source: Box::new(source),
            }),
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
                    .map_err(|source| SandboxError::Docker {
                        operation: "starting an existing server container",
                        source: Box::new(source),
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
            .map_err(|source| SandboxError::Docker {
                operation: "creating a server container",
                source: Box::new(source),
            })?;

        self.client
            .docker
            .start_container(&server_container_name, None::<StartContainerOptions>)
            .await
            .map_err(|source| SandboxError::Docker {
                operation: "starting a server container",
                source: Box::new(source),
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
    async fn list_server_container_names(
        &self,
        task_id: &str,
    ) -> Result<Vec<String>, SandboxError> {
        use std::collections::HashMap;

        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![
                format!("moatless.io/task-id={task_id}"),
                format!(
                    "{}={}",
                    container_builder::SERVICE_LABEL,
                    container_builder::SERVER_SERVICE
                ),
            ],
        );

        self.client
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::new()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map(|containers| {
                containers
                    .into_iter()
                    .filter_map(|c| {
                        c.names
                            .and_then(|names| names.into_iter().next())
                            .map(|name| name.trim_start_matches('/').to_string())
                    })
                    .collect()
            })
            .map_err(|source| SandboxError::Docker {
                operation: "listing server containers",
                source: Box::new(source),
            })
    }

    /// Stop all server containers for a task.
    async fn stop_server_containers(&self, task_id: &str) {
        let names = match self.list_server_container_names(task_id).await {
            Ok(names) => names,
            Err(e) => {
                warn!(task_id, error = %e, "Cannot stop server containers after list failure");
                return;
            }
        };
        for name in names {
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
        let names = match self.list_server_container_names(task_id).await {
            Ok(names) => names,
            Err(e) => {
                warn!(task_id, error = %e, "Cannot restart server containers after list failure");
                return;
            }
        };
        for name in names {
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

    /// Force-remove all server containers and verify their label set is empty.
    async fn remove_server_containers(&self, task_id: &str) -> Result<(), SandboxError> {
        let names = self.list_server_container_names(task_id).await?;
        self.remove_named_server_containers(task_id, names).await
    }

    async fn remove_named_server_containers(
        &self,
        task_id: &str,
        names: Vec<String>,
    ) -> Result<(), SandboxError> {
        for name in names {
            self.force_remove_container(&name).await?;
        }
        if self.list_server_container_names(task_id).await?.is_empty() {
            Ok(())
        } else {
            Err(SandboxError::DockerError(format!(
                "server containers for task {task_id} remain after removal"
            )))
        }
    }

    /// Force-remove a container. `Ok` on success or when it is already gone
    /// (404); a non-404 error propagates so removal can be load-bearing —
    /// strict cleanup and edge teardown must not proceed past a failed remove.
    async fn force_remove_container(&self, container_name: &str) -> Result<(), SandboxError> {
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
                status_code: 404, ..
            }) => Ok(()),
            Err(source) => Err(SandboxError::Docker {
                operation: "removing a container",
                source: Box::new(source),
            }),
        }
    }

    async fn container_exists(&self, container_name: &str) -> Result<bool, SandboxError> {
        match self
            .client
            .docker
            .inspect_container(container_name, None)
            .await
        {
            Ok(_) => Ok(true),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(false),
            Err(source) => Err(SandboxError::Docker {
                operation: "inspecting a container by name",
                source: Box::new(source),
            }),
        }
    }

    async fn ensure_container_absent(
        &self,
        container_name: &str,
        task_id: &str,
    ) -> Result<(), SandboxError> {
        if self.container_exists(container_name).await? {
            Err(SandboxError::NotReady(format!(
                "container {container_name} for task {task_id} remains after removal"
            )))
        } else {
            Ok(())
        }
    }

    async fn ensure_container_absent_runtime(
        &self,
        container_name: &str,
        task_id: &str,
    ) -> Result<(), moatless_sandbox_control::lifecycle::RuntimeError> {
        if self
            .container_exists(container_name)
            .await
            .map_err(map_sandbox_error)?
        {
            Err(
                moatless_sandbox_control::lifecycle::RuntimeError::NotAbsent(format!(
                    "container {container_name} for task {task_id}"
                )),
            )
        } else {
            Ok(())
        }
    }

    async fn delete_archive_prefix(
        &self,
        task_id: &str,
        root: &str,
        kind: &str,
    ) -> Result<(), moatless_sandbox_control::lifecycle::RuntimeError> {
        let (Some(s3), Some(bucket)) = (self.s3.as_ref(), self.config.s3.bucket_name.as_deref())
        else {
            return Ok(());
        };
        let prefix = format!("{}/{task_id}/", root.trim_end_matches('/'));
        moatless_s3::delete_objects_by_prefix(s3, bucket, &prefix)
            .await
            .map_err(|e| {
                moatless_sandbox_control::lifecycle::RuntimeError::Infrastructure(format!(
                    "delete {kind} archive for {task_id}: {e}"
                ))
            })?;
        Ok(())
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
    use moatless_sandbox_control::lifecycle::OperationGeneration;
    use moatless_sandbox_control::lifecycle::runtime::OPERATION_GENERATION_LABEL;
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
        assert_eq!(
            rebuilt.env.as_deref(),
            Some(&["TASK_ID=t1".to_string()][..])
        );
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
        assert!(
            rebuilt
                .networking_config
                .and_then(|networking| networking.endpoints_config)
                .is_some_and(|networks| networks.contains_key("preview"))
        );
    }

    #[tokio::test]
    async fn docker_retain_preserves_task_directory() {
        let root = tempfile::tempdir().expect("temp dir");
        let root = root.path().to_string_lossy().to_string();
        let task_path = generate_task_host_path(&root, "task-1").expect("task path");
        tokio::fs::create_dir_all(task_path.join("repo"))
            .await
            .expect("create task data");
        tokio::fs::write(task_path.join("repo/evidence.txt"), b"retained")
            .await
            .expect("write evidence");

        apply_disk_archive_retention(&root, "task-1", ArchiveRetention::RetainArchives)
            .await
            .expect("retain");

        assert!(
            tokio::fs::try_exists(task_path.join("repo/evidence.txt"))
                .await
                .expect("stat evidence")
        );
    }

    #[tokio::test]
    async fn docker_delete_removes_task_directory_idempotently() {
        let root = tempfile::tempdir().expect("temp dir");
        let root = root.path().to_string_lossy().to_string();
        let task_path = generate_task_host_path(&root, "task-1").expect("task path");
        tokio::fs::create_dir_all(task_path.join("claude-config"))
            .await
            .expect("create task data");
        tokio::fs::write(task_path.join("claude-config/transcript.jsonl"), b"deleted")
            .await
            .expect("write transcript");

        for _ in 0..2 {
            apply_disk_archive_retention(&root, "task-1", ArchiveRetention::DeleteArchives)
                .await
                .expect("delete is idempotent");
        }

        assert!(
            !tokio::fs::try_exists(&task_path)
                .await
                .expect("stat task directory")
        );
    }

    #[tokio::test]
    async fn docker_delete_propagates_filesystem_failure() {
        let root = tempfile::tempdir().expect("temp dir");
        let root = root.path().to_string_lossy().to_string();
        let task_path = generate_task_host_path(&root, "task-1").expect("task path");
        tokio::fs::create_dir_all(task_path.parent().expect("tasks parent"))
            .await
            .expect("create tasks directory");
        tokio::fs::write(&task_path, b"not a directory")
            .await
            .expect("create blocking file");

        let error = apply_disk_archive_retention(&root, "task-1", ArchiveRetention::DeleteArchives)
            .await
            .expect_err("filesystem failure must propagate");

        assert!(matches!(
            error,
            moatless_sandbox_control::lifecycle::RuntimeError::Infrastructure(_)
        ));
    }
}
