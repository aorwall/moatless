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
use super::health;
use super::naming::generate_container_name;
use super::status_resolver::DockerStatusResolver;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::users::UserRepository;
use crate::config::Config;
use crate::events::EventPublisher;
use crate::repository::dao::RepoRepository;
use crate::sandbox::error::SandboxError;
use crate::sandbox::grpc_client::SandboxGrpcClient;
use crate::sandbox::jwt;
use crate::sandbox::k8s::auto_provision;
use crate::sandbox::models::*;
use crate::sandbox::provider::{SandboxProvider, StartResult};
use crate::sandbox::schemas::*;
use crate::task::dao::TaskRepository;

/// Docker-based sandbox provider.
pub struct DockerSandboxProvider {
    pub config: Arc<Config>,
    pub client: DockerClient,
    pub status_resolver: DockerStatusResolver,
    /// Optional user repository — enables user login resolution on `/provision`.
    pub users: Option<Arc<dyn UserRepository>>,
    /// Optional task repository — enables auto-provisioning on `/start`.
    pub tasks: Option<Arc<dyn TaskRepository>>,
    /// Optional repo repository — enables auto-provisioning on `/start`.
    pub repos: Option<Arc<dyn RepoRepository>>,
    /// Optional provider token repository — enables credential resolution.
    pub provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
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

        // 1. Check for existing running container
        match self.check_existing_container(&container_name).await {
            Ok(true) => {
                info!(task_id = %task_id, "Reusing existing running container");
                return Ok(ProvisionResponse {
                    status: EnvironmentStatus::Ready,
                });
            }
            Ok(false) => {} // No existing container, continue provisioning
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Error checking existing container");
            }
        }

        // 2. Resolve user login for JWT generation
        let task_token_secret = self
            .config
            .sandbox
            .task_token_secret
            .as_ref()
            .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?
            .clone();

        let user_login = if let Some(ref users) = self.users {
            match users.get_user(&request.user_id).await {
                Ok(Some(user)) => user.login,
                _ => request.user_id.clone(),
            }
        } else {
            request.user_id.clone()
        };

        let user_auth_token =
            jwt::generate_user_auth_token(&request.user_id, &user_login, &task_token_secret)
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

        // 5. Remove any existing stopped container with the same name
        let _ = self
            .client
            .docker
            .remove_container(
                &container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        // 6. Create and start container
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

        // Publish initializing event
        self.publish_initializing(task_id).await;

        Ok(ProvisionResponse {
            status: EnvironmentStatus::Initializing,
        })
    }

    async fn status(&self, task_id: &str) -> Result<StatusResponse, SandboxError> {
        Ok(self.status_resolver.resolve_status(task_id).await)
    }

    async fn sandbox_status(
        &self,
        task_id: &str,
    ) -> Result<WorkspaceStatusFullResponse, SandboxError> {
        Ok(self.status_resolver.resolve_sandbox_status(task_id).await)
    }

    async fn health(&self, task_id: &str) -> HealthResponse {
        self.status_resolver.resolve_health(task_id).await
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

    async fn start(&self, task_id: &str) -> Result<StartResult, SandboxError> {
        info!(task_id = %task_id, "Starting Docker sandbox");

        let container_name = generate_container_name(task_id);

        // Check if container exists
        match self.client.docker.inspect_container(&container_name, None).await {
            Ok(info) => {
                let status_str = info
                    .state
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();

                if status_str == "running" {
                    return Ok(StartResult::AlreadyRunning);
                }

                // Container exists but not running — start it
                self.client
                    .docker
                    .start_container(&container_name, None::<StartContainerOptions<String>>)
                    .await
                    .map_err(|e| {
                        SandboxError::DockerError(format!("Failed to start container: {e}"))
                    })?;

                self.publish_initializing(task_id).await;

                Ok(StartResult::Starting)
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                // No container — try auto-provisioning from DB
                info!(task_id = %task_id, "No container found, attempting auto-provision from DB");
                self.auto_provision(task_id).await?;
                Ok(StartResult::Starting)
            }
            Err(e) => Err(SandboxError::DockerError(format!(
                "Failed to inspect container: {e}"
            ))),
        }
    }

    async fn restart(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Restarting Docker sandbox");

        let container_name = generate_container_name(task_id);

        self.client
            .docker
            .restart_container(&container_name, Some(RestartContainerOptions { t: 30 }))
            .await
            .map_err(|e| {
                SandboxError::DockerError(format!("Failed to restart container: {e}"))
            })?;

        self.publish_initializing(task_id).await;

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

    async fn server_status(
        &self,
        _task_id: &str,
    ) -> Result<ServerStatusResponse, SandboxError> {
        // Docker mode is agent-only — no server management
        Ok(ServerStatusResponse { servers: vec![] })
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

    async fn search_files(
        &self,
        task_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<FileSearchResult>, SandboxError> {
        self.require_sandbox_ready(task_id).await?;
        let container_name = generate_container_name(task_id);
        let results = health::search_sandbox_files_docker(
            &container_name,
            self.config.sandbox.grpc_port,
            query,
            limit,
        )
        .await
        .map_err(SandboxError::Internal)?;

        Ok(results)
    }

    // ── File operations ──────────────────────────────────────────

    async fn get_file_tree(
        &self,
        task_id: &str,
        root_path: &str,
    ) -> Result<FileTreeResponse, SandboxError> {
        let client = self.ready_grpc_client(task_id).await?;
        let resp = client
            .get_file_tree(root_path)
            .await
            .map_err(|e| Self::classify_grpc_error(e, "get_file_tree"))?;

        let summary = resp.git_summary.map_or(
            GitSummaryDto {
                total_files: 0,
                total_additions: 0,
                total_deletions: 0,
                total_staged: 0,
                total_modified: 0,
                compare_base: "HEAD".to_string(),
                has_changes: false,
            },
            GitSummaryDto::from,
        );

        Ok(FileTreeResponse {
            tree: resp.tree.into_iter().map(FileTreeNodeDto::from).collect(),
            git_summary: summary,
        })
    }

    async fn read_file(
        &self,
        task_id: &str,
        path: &str,
        include_changes: bool,
    ) -> Result<FileContentResponse, SandboxError> {
        let client = self.ready_grpc_client(task_id).await?;
        let resp = client
            .read_file(path, include_changes)
            .await
            .map_err(|e| Self::classify_grpc_error(e, "read_file"))?;

        Ok(FileContentResponse::from(resp))
    }

    async fn write_file(
        &self,
        task_id: &str,
        path: &str,
        content: &str,
    ) -> Result<(), SandboxError> {
        let client = self.ready_grpc_client(task_id).await?;
        client
            .write_file(path, content)
            .await
            .map_err(|e| Self::classify_grpc_error(e, "write_file"))?;
        Ok(())
    }

    async fn list_files(
        &self,
        task_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<ListFilesResponse, SandboxError> {
        let client = self.ready_grpc_client(task_id).await?;
        let files = client
            .list_files(path, recursive)
            .await
            .map_err(|e| Self::classify_grpc_error(e, "list_files"))?;
        Ok(ListFilesResponse { files })
    }

    async fn search_files_by_name(
        &self,
        task_id: &str,
        query: &str,
        case_sensitive: bool,
        max_results: i32,
    ) -> Result<SearchByNameResponse, SandboxError> {
        let client = self.ready_grpc_client(task_id).await?;
        let resp = client
            .search_files_by_name(query, case_sensitive, max_results)
            .await
            .map_err(|e| Self::classify_grpc_error(e, "search_files_by_name"))?;

        Ok(SearchByNameResponse {
            files: resp
                .files
                .into_iter()
                .map(FileNameMatchDto::from)
                .collect(),
            total_matches: resp.total_matches,
        })
    }
}

// =============================================================================
// Private helper methods
// =============================================================================

impl DockerSandboxProvider {
    /// Create a gRPC client for a sandbox container.
    fn grpc_client(&self, task_id: &str) -> Result<SandboxGrpcClient, SandboxError> {
        let container_name = generate_container_name(task_id);
        let url = format!(
            "http://{}:{}",
            container_name, self.config.sandbox.grpc_port
        );
        SandboxGrpcClient::connect(&url)
            .map_err(|e| SandboxError::Internal(format!("Failed to create gRPC client: {e}")))
    }

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

    /// Get a gRPC client after verifying the sandbox is running.
    async fn ready_grpc_client(
        &self,
        task_id: &str,
    ) -> Result<SandboxGrpcClient, SandboxError> {
        self.require_sandbox_ready(task_id).await?;
        self.grpc_client(task_id)
    }

    /// Classify gRPC errors: Unavailable/connection errors → NotReady (503),
    /// everything else → Internal (500).
    fn classify_grpc_error(e: tonic::Status, operation: &str) -> SandboxError {
        match e.code() {
            tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
                SandboxError::NotReady(format!("Sandbox unreachable: {operation}"))
            }
            _ => SandboxError::Internal(format!("gRPC {operation} failed: {e}")),
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

    /// Check if an existing container is running and healthy.
    async fn check_existing_container(
        &self,
        container_name: &str,
    ) -> Result<bool, String> {
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

                Ok(status_str == "running")
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                ..
            }) => Ok(false),
            Err(e) => Err(format!("Failed to inspect container: {e}")),
        }
    }

    /// Auto-provision a workspace from DB state when no container exists.
    async fn auto_provision(&self, task_id: &str) -> Result<(), SandboxError> {
        let tasks = self.tasks.as_ref().ok_or_else(|| {
            SandboxError::NotReady(
                "Workspace not provisioned and DATABASE_URL not configured for auto-provisioning. \
                 Use /provision first."
                    .to_string(),
            )
        })?;
        let repos = self.repos.as_ref().ok_or_else(|| {
            SandboxError::NotReady(
                "Repository access not configured for auto-provisioning.".to_string(),
            )
        })?;
        let provider_tokens = self.provider_tokens.as_ref().ok_or_else(|| {
            SandboxError::NotReady(
                "Provider tokens not configured for auto-provisioning.".to_string(),
            )
        })?;

        // Note: auto_provision::build_provision_request takes a K8sClient param
        // that is unused (_k8s). We pass a dummy reference by using a helper that
        // skips it. For now, we construct the request manually.
        let request = self
            .build_provision_request_from_db(
                tasks.as_ref(),
                repos.as_ref(),
                provider_tokens.as_ref(),
                task_id,
            )
            .await?;

        info!(
            task_id = %task_id,
            repo_name = %request.plan.repo_config.name,
            "Auto-provisioning Docker sandbox from DB"
        );

        self.provision(task_id, &request).await?;

        info!(task_id = %task_id, "Auto-provision succeeded");
        Ok(())
    }

    /// Build a ProvisionRequest from database state (without K8s dependency).
    ///
    /// This duplicates the logic from `auto_provision::build_provision_request`
    /// but without the `K8sClient` parameter.
    async fn build_provision_request_from_db(
        &self,
        tasks: &dyn TaskRepository,
        repos: &dyn RepoRepository,
        provider_tokens: &dyn ProviderTokenRepository,
        task_id: &str,
    ) -> Result<ProvisionRequest, SandboxError> {
        let task = tasks
            .find_by_id_internal(task_id)
            .await
            .map_err(|e| SandboxError::Internal(format!("DB error reading task: {e}")))?
            .ok_or_else(|| {
                SandboxError::Internal(format!("Task '{task_id}' not found in database"))
            })?;

        let repo = repos
            .find_repository_by_id(&task.repository_id)
            .await
            .map_err(|e| SandboxError::Internal(format!("DB error reading repository: {e}")))?
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "Repository '{}' not found in database",
                    task.repository_id
                ))
            })?;

        // Resolve credentials from provider_tokens + env vars
        let user_id = task.created_by.as_deref();
        let (git_token, anthropic, claude) =
            resolve_credentials_without_k8s(provider_tokens, task_id, user_id).await;

        let git_clone_url = repo.remote_url.clone().unwrap_or_default();
        let git_branch = task.branch.or(repo.default_branch.clone());
        let repo_config = auto_provision::build_server_configs(&repo);

        // Build repo config from repository row
        let servers = if repo_config.is_empty() {
            None
        } else {
            Some(repo_config)
        };

        let plan = ProvisionPlan {
            repo_config: RepoConfig {
                name: repo.name.clone(),
                docker_image: repo.docker_image.clone(),
                servers,
                plugin_ids: repo
                    .plugins
                    .as_ref()
                    .and_then(|v| serde_json::from_value(v.clone()).ok()),
                system_prompt: repo.system_prompt.clone(),
                code_execution: None,
                preview_config: None,
            },
            git_clone_url,
            git_branch,
            git_provider_token: git_token,
            claude_oauth_token: claude,
            anthropic_api_key: anthropic,
            env_vars: repo
                .env_vars
                .as_ref()
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
        };

        let request_user_id = task.created_by.unwrap_or_else(|| "system".to_string());
        Ok(ProvisionRequest {
            plan,
            user_id: request_user_id,
        })
    }
}

/// Resolve credentials from provider_tokens + env vars (no K8s dependency).
async fn resolve_credentials_without_k8s(
    provider_tokens: &dyn ProviderTokenRepository,
    task_id: &str,
    user_id: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    let mut git_token: Option<String> = None;
    let mut anthropic: Option<String> = None;
    let mut claude: Option<String> = None;

    if let Some(uid) = user_id {
        if let Ok(Some(row)) = provider_tokens.find_provider_token(uid, "github").await {
            info!(task_id = %task_id, "Auto-provision: GitHub token resolved from provider_tokens");
            git_token = Some(row.access_token);
        }
        if let Ok(Some(row)) = provider_tokens.find_provider_token(uid, "claude").await {
            info!(task_id = %task_id, "Auto-provision: Claude token resolved from provider_tokens");
            claude = Some(row.access_token);
        }
    }

    if git_token.is_none() {
        git_token = std::env::var("GITHUB_PAT").ok();
    }
    if anthropic.is_none() {
        anthropic = std::env::var("ANTHROPIC_API_KEY").ok();
    }
    if claude.is_none() {
        claude = std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok();
    }

    (git_token, anthropic, claude)
}
