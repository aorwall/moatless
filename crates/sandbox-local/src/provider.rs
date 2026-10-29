//! Local-process sandbox runtime, observer, and connector adapter.

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use std::sync::Arc;
use tracing::{info, warn};

use super::client::LocalClient;
use super::command_builder;
use super::naming;
use super::status_resolver::LocalStatusResolver;
use moatless_core::config::Config;
use moatless_core::sandbox_jwt as jwt;
use moatless_ports::sandbox as core_sandbox;
use moatless_sandbox_control::lifecycle::ProvisionDeps;
use moatless_sandbox_control::status;
use moatless_sandbox_runtime::client::SandboxClient;
use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::operation::{ObservedStatus, RuntimeEffect};
use moatless_sandbox_runtime::outcome::{DegradationWarning, Diagnostics};
use moatless_sandbox_runtime::provisioning::ProvisionRequest;
use moatless_sandbox_runtime::runtime::{
    RuntimeOperationContext, RuntimeOutcome, SandboxRuntime, map_sandbox_error,
};
use moatless_sandbox_runtime::schemas::*;
use moatless_sandbox_runtime::status_reader::SandboxStatusReader;
use moatless_types::sandbox::ArchiveRetention;

/// Local-process adapter for lifecycle mutation, observation, and connection.
pub struct LocalSandboxAdapter {
    config: Arc<Config>,
    client: LocalClient,
    status_resolver: LocalStatusResolver,
    plan: Option<ProvisionDeps>,
}

impl LocalSandboxAdapter {
    /// Construct the local adapter over the process registry and plan
    /// resolver.
    pub fn new(config: Arc<Config>, client: LocalClient, plan: Option<ProvisionDeps>) -> Self {
        let status_resolver = LocalStatusResolver::new(client.clone());
        Self {
            config,
            client,
            status_resolver,
            plan,
        }
    }

    fn require_plan(
        &self,
    ) -> Result<&ProvisionDeps, moatless_sandbox_runtime::runtime::RuntimeError> {
        self.plan.as_ref().ok_or_else(|| {
            moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure(
                "sandbox plan repositories unavailable".to_string(),
            )
        })
    }

    async fn spawn_sandbox(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, SandboxError> {
        info!(task_id = %task_id, user_id = %request.user_id, "Spawning local sandbox process");

        let mut diagnostics = Diagnostics::default();
        let plan = &request.plan;

        // 1. Mint a user auth token, exactly like the Docker path.
        let task_token_secret = self
            .config
            .sandbox
            .task_token_secret
            .as_ref()
            .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?
            .clone();
        let user_auth_token = jwt::generate_user_auth_token(
            &request.user_id,
            &request.user_id,
            task_id,
            request.token_generation,
            &task_token_secret,
            self.config.sandbox.user_token_ttl_secs,
        )
        .map_err(|source| SandboxError::JwtSigning {
            purpose: "the sandbox user auth token",
            source,
        })?;

        let paths =
            naming::task_paths(self.client.root(), task_id).map_err(SandboxError::BadRequest)?;
        for dir in paths.dirs_to_create() {
            if let Err(e) = tokio::fs::create_dir_all(dir).await {
                warn!(
                    task_id = %task_id,
                    path = %dir.display(),
                    error = %e,
                    "Failed to create directory (may already exist)"
                );
                diagnostics.degradations.push(DegradationWarning {
                    code: "host_directory_unavailable".to_string(),
                    message: format!("Failed to prepare {}: {e}", dir.display()),
                });
            }
        }

        let (http_port, grpc_port) = LocalClient::allocate_ports()
            .await
            .map_err(|e| SandboxError::Internal(format!("allocate local sandbox ports: {e}")))?;
        let env = command_builder::build_env(
            &self.config,
            task_id,
            plan,
            Some(&user_auth_token),
            &paths,
            http_port,
            grpc_port,
        );

        self.client
            .spawn(
                task_id,
                &env,
                &paths.log_file(),
                http_port,
                grpc_port,
                Some(ctx.operation_generation),
            )
            .await
            .map_err(|e| SandboxError::Internal(format!("spawn local sandbox process: {e}")))?;

        info!(
            task_id = %task_id,
            http_port,
            grpc_port,
            workspace_dir = %paths.workspace_dir.display(),
            "Local sandbox process spawned"
        );

        Ok(RuntimeOutcome {
            observed: ObservedStatus::Ready,
            effect: RuntimeEffect::Applied,
            diagnostics,
        })
    }
}

#[async_trait]
impl SandboxRuntime for LocalSandboxAdapter {
    async fn ensure_running(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        let plan = self.require_plan()?;
        let request = plan.build_request(task_id).await?;

        if let Some(snapshot) = self.client.snapshot(task_id).await {
            if snapshot.alive {
                return Ok(RuntimeOutcome {
                    observed: ObservedStatus::Ready,
                    effect: RuntimeEffect::NoOp,
                    diagnostics: Diagnostics::default(),
                });
            }
            // Stale (exited) entry — clear it before respawning.
            self.client.purge(task_id).await;
        }

        self.spawn_sandbox(task_id, &request, ctx)
            .await
            .map_err(map_sandbox_error)
    }

    async fn stop(
        &self,
        task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        info!(task_id = %task_id, "Stopping local sandbox process");
        let was_alive = self.client.kill(task_id).await;
        Ok(RuntimeOutcome {
            observed: ObservedStatus::Stopped,
            effect: if was_alive {
                RuntimeEffect::Applied
            } else {
                RuntimeEffect::NoOp
            },
            diagnostics: Diagnostics::default(),
        })
    }

    async fn restart(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        info!(task_id = %task_id, "Restarting local sandbox process");
        let plan = self.require_plan()?;
        let request = plan.build_request(task_id).await?;
        self.client.purge(task_id).await;
        self.spawn_sandbox(task_id, &request, ctx)
            .await
            .map_err(map_sandbox_error)
    }

    async fn redeploy(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        info!(task_id = %task_id, "Redeploying local sandbox process");
        let plan = self.require_plan()?;
        let request = plan.build_request(task_id).await?;
        self.client.purge(task_id).await;
        self.spawn_sandbox(task_id, &request, ctx)
            .await
            .map_err(map_sandbox_error)
    }

    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        info!(task_id = %task_id, "Removing local sandbox process");
        let had_runtime = self.client.snapshot(task_id).await.is_some_and(|s| s.alive);
        self.client.purge(task_id).await;

        if !retention.retains() {
            let base = naming::task_base_dir(self.client.root(), task_id)
                .map_err(moatless_sandbox_runtime::runtime::RuntimeError::Rejected)?;
            match tokio::fs::remove_dir_all(&base).await {
                Ok(()) => {
                    info!(task_id = %task_id, path = %base.display(), "Deleted local task directory");
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(
                        moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure(format!(
                            "delete local task directory {}: {e}",
                            base.display()
                        )),
                    );
                }
            }
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
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        // The local runtime has no separable storage tier to fall down from.
        tracing::debug!(task_id = %task_id, "Storage reclaim is a no-op on the local runtime");
        Ok(RuntimeOutcome {
            observed: ObservedStatus::Stopped,
            effect: RuntimeEffect::NoOp,
            diagnostics: Diagnostics::default(),
        })
    }
}

#[async_trait]
impl SandboxStatusReader for LocalSandboxAdapter {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: moatless_types::task::TaskStatus,
        desired_state: moatless_types::sandbox::SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        status::sandbox_status(
            &self.status_resolver,
            self.plan.as_ref(),
            self,
            task_id,
            task_status,
            desired_state,
        )
        .await
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: moatless_types::task::TaskStatus,
        desired_state: moatless_types::sandbox::SandboxDesiredState,
    ) -> Result<moatless_sandbox_runtime::live_status::LiveStatus, SandboxError> {
        status::live_status(
            &self.status_resolver,
            self.plan.as_ref(),
            self,
            task_id,
            task_status,
            desired_state,
        )
        .await
    }

    async fn sandbox_logs(
        &self,
        task_id: &str,
        options: moatless_sandbox_runtime::access::LogOptions,
    ) -> Result<BoxStream<'static, Result<String, SandboxError>>, SandboxError> {
        let sandbox_id = self.sandbox_id_for_task(task_id).await?;
        let base = naming::task_base_dir(self.client.root(), &sandbox_id)
            .map_err(SandboxError::BadRequest)?;
        let log_path = base.join("sandbox.log");
        let contents = tokio::fs::read_to_string(&log_path)
            .await
            .map_err(|e| SandboxError::NotFound(format!("no local sandbox log yet: {e}")))?;

        let tail_lines = usize::try_from(options.tail_lines).unwrap_or(0);
        let mut lines: Vec<String> = contents
            .lines()
            .rev()
            .take(tail_lines)
            .map(std::string::ToString::to_string)
            .collect();
        lines.reverse();

        Ok(Box::pin(stream::iter(lines.into_iter().map(Ok))))
    }
}

#[async_trait]
impl moatless_sandbox_runtime::client::SandboxConnector for LocalSandboxAdapter {
    async fn sandbox_client(
        &self,
        task_id: &str,
    ) -> Result<SandboxClient, core_sandbox::SandboxError> {
        let sandbox_id = self
            .sandbox_id_for_task(task_id)
            .await
            .map_err(SandboxError::into_contract)?;
        let snapshot = self.client.snapshot(&sandbox_id).await.ok_or_else(|| {
            core_sandbox::SandboxError::Other("local sandbox not provisioned".to_string())
        })?;
        if !snapshot.alive {
            return Err(core_sandbox::SandboxError::Other(
                "local sandbox process is not running".to_string(),
            ));
        }
        let url = format!("http://127.0.0.1:{}", snapshot.grpc_port);
        SandboxClient::connect(&url)
            .map_err(|e| core_sandbox::SandboxError::GrpcClient(e.to_string()))
    }
}

impl LocalSandboxAdapter {
    async fn sandbox_id_for_task(&self, task_id: &str) -> Result<String, SandboxError> {
        status::sandbox_id_for_task(self.plan.as_ref(), task_id).await
    }
}

/// The local runtime binds no reverse proxy or ingress, so it has no way to
/// publish a task-scoped preview port.
#[async_trait]
impl core_sandbox::SandboxPortPublisher for LocalSandboxAdapter {
    async fn publish_port(
        &self,
        task_id: &str,
        _name: &str,
        _port: u16,
    ) -> Result<Option<String>, core_sandbox::SandboxError> {
        tracing::debug!(
            task_id,
            "Local sandbox mode does not support publishing preview ports"
        );
        Err(core_sandbox::SandboxError::Unsupported {
            operation: "publish_port".to_string(),
            source: Box::<dyn std::error::Error + Send + Sync>::from(
                "the local-process runtime has no reverse proxy",
            ),
        })
    }
}

/// The local runtime has no cluster-wide view beyond its own in-memory
/// registry, and nothing outside this one backend process could observe it
/// anyway.
#[async_trait]
impl core_sandbox::ClusterDiagnostics for LocalSandboxAdapter {
    async fn list_sandbox_diagnostics(
        &self,
    ) -> Result<Vec<core_sandbox::SandboxDiagnosticsEntry>, core_sandbox::SandboxError> {
        Ok(Vec::new())
    }
}
