//! Provider-agnostic auto-provisioning logic.
//!
//! Reads task + repository from DB, resolves credentials, and builds
//! a `ProvisionRequest`. Used by handlers and other modules that need
//! to ensure a sandbox is running without knowing provisioning details.

use tracing::{info, warn};

use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::plugin_repo::dao::PluginRepoRepository;
use crate::repository::dao::RepoRepository;
use crate::repository::models::RepositoryProvider;
use crate::sandbox::error::SandboxError;
use crate::sandbox::models::{ProvisionRequest, ServerConfig};
use crate::task::dao::TaskRepository;
use crate::task::models::Task;
use crate::workspace::dao::WorkspaceRepository;

// Re-export the repo-config builders from k8s/auto_provision so existing
// callers can use the same path.
pub use crate::sandbox::k8s::auto_provision::{
    build_repo_config, build_server_configs, filter_disabled_servers, get_server_configs,
    parse_disabled_servers,
};

/// Build a `ProvisionRequest` from database state.
///
/// Provider-agnostic: works for both K8s and Docker sandboxes.
///
/// Steps:
/// 1. Read the task row to get the workspace/repository reference + branch
/// 2. Resolve the primary repository (via the workspace when present) and read
///    its row to get repo config
/// 3. Assemble the ProvisionRequest. Runtime credentials are fetched later
///    over gRPC by the sandbox.
pub async fn build_provision_request(
    tasks: &dyn TaskRepository,
    repos: &dyn RepoRepository,
    _provider_tokens: &dyn ProviderTokenRepository,
    plugin_repos: &dyn PluginRepoRepository,
    workspaces: &dyn WorkspaceRepository,
    task_id: &str,
) -> Result<ProvisionRequest, SandboxError> {
    // 1. Read task from DB
    let task = tasks
        .find_by_id_internal(task_id)
        .await
        .map_err(|e| SandboxError::Internal(format!("DB error reading task: {e}")))?
        .ok_or_else(|| SandboxError::Internal(format!("Task '{task_id}' not found in database")))?;

    // Gate: never provision a pod for a Closed task. A webhook racing a close
    // (Fix D commits `Closed` first) lands here and is rejected, so the close
    // can no longer be undone by a re-provision — the direct fix for the
    // "close started it again" race. Only `Closed` is gated: `Error` stays
    // resumable to match `start_task`.
    if task.status == crate::task::models::TaskStatus::Closed {
        return Err(SandboxError::BadRequest(format!(
            "Task '{task_id}' is closed; refusing to provision a sandbox"
        )));
    }

    // Resolve which repository this task provisions. When the task references a
    // workspace (Phase A backfills a shadow workspace per repo), use the
    // workspace's primary repo; otherwise fall back to the legacy
    // `repository_id`. In Phase A the shadow workspace's primary repo equals
    // `repository_id`, so the single-repo provisioning path is unchanged.
    let repository_id = resolve_primary_repository_id(workspaces, &task).await;

    info!(
        task_id = %task_id,
        repository_id = %repository_id,
        workspace_id = ?task.workspace_id,
        "Auto-provision: task found in DB"
    );

    // 2. Read repository from DB
    let repo = repos
        .find_repository_by_id(&repository_id)
        .await
        .map_err(|e| SandboxError::Internal(format!("DB error reading repository: {e}")))?
        .ok_or_else(|| {
            SandboxError::Internal(format!(
                "Repository '{repository_id}' not found in database"
            ))
        })?;

    let git_provider = repo.provider.unwrap_or(RepositoryProvider::Github);

    let git_clone_url = repo.remote_url.clone().unwrap_or_default();
    let git_branch = task.branch.clone().or(repo.default_branch.clone());
    let user_id = task.created_by.as_deref().unwrap_or("system").to_string();

    let mut repo_config = build_repo_config(&repo);
    let mut env_vars = crate::sandbox::k8s::auto_provision::parse_env_vars(&repo);

    // Select the tmux Claude transport per the catalog when the task's agent
    // requests it. Each task is its own pod, so the pod-level flag is exactly
    // the right granularity.
    if crate::agent::catalog::use_tmux_for(task.agent_type.0.as_str()) {
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("SANDBOX_USE_TMUX".to_string(), "true".to_string());
    }

    // Resolve plugin git repos from the join table
    match plugin_repos
        .find_by_repository_id(&repository_id)
        .await
    {
        Ok(plugins) if !plugins.is_empty() => {
            let urls: Vec<String> = plugins.into_iter().map(|p| p.git_url).collect();
            info!(
                task_id = %task_id,
                count = urls.len(),
                "Auto-provision: resolved plugin git repos"
            );
            repo_config.plugin_repos = Some(urls);
        }
        Ok(_) => {}
        Err(e) => {
            warn!(
                task_id = %task_id,
                error = %e,
                "Auto-provision: failed to query plugin repos (non-fatal)"
            );
        }
    }

    if task.headless {
        repo_config.servers = None;
        repo_config.preview_config = None;
    }

    // Drop servers this task previously auto-disabled (crash-loop remediation).
    // Persisted per-task so it survives stop/start and never affects sibling
    // tasks sharing the same repository.
    let disabled = parse_disabled_servers(&task);
    if !disabled.is_empty() {
        info!(task_id = %task_id, ?disabled, "Auto-provision: filtering disabled servers");
        filter_disabled_servers(&mut repo_config, &disabled);
    }

    let plan = crate::sandbox::models::ProvisionPlan {
        repo_config,
        git_clone_url,
        git_branch,
        git_provider: git_provider.as_str().to_string(),
        env_vars,
    };

    Ok(ProvisionRequest { plan, user_id })
}

/// Resolve the repository a task provisions against: the workspace's primary
/// repo when `task.workspace_id` is set, else the legacy `task.repository_id`.
///
/// Falls back to `repository_id` when the workspace is empty or the lookup
/// fails, so provisioning is never blocked by workspace-resolution issues. In
/// Phase A a task's shadow workspace has exactly one primary repo equal to
/// `repository_id`, so the resolved repository is unchanged.
async fn resolve_primary_repository_id(
    workspaces: &dyn WorkspaceRepository,
    task: &Task,
) -> String {
    let Some(workspace_id) = task.workspace_id.as_deref() else {
        return task.repository_id.clone();
    };
    match workspaces.find_primary_repo(workspace_id).await {
        Ok(Some(placement)) => placement.repository_id,
        Ok(None) => {
            warn!(
                task_id = %task.task_id,
                workspace_id,
                "Auto-provision: workspace has no repos; falling back to task.repository_id"
            );
            task.repository_id.clone()
        }
        Err(e) => {
            warn!(
                task_id = %task.task_id,
                workspace_id,
                error = %e,
                "Auto-provision: failed to resolve workspace primary repo; \
                 falling back to task.repository_id"
            );
            task.repository_id.clone()
        }
    }
}

/// Get server configs for a task, returning an empty vec on any failure.
///
/// Convenience wrapper that handles the Option<Arc<dyn Repo>> pattern
/// used throughout handlers.
pub async fn get_server_configs_for_task(
    tasks: &dyn TaskRepository,
    repos: &dyn RepoRepository,
    task_id: &str,
) -> Vec<ServerConfig> {
    match get_server_configs(tasks, repos, task_id).await {
        Ok(configs) => configs,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to get server configs");
            vec![]
        }
    }
}
