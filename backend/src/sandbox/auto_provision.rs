//! Provider-agnostic lifecycle plan construction and pod-watcher config reads.
//!
//! Lifecycle uses [`build_provision_request`] to assemble a fresh runtime plan
//! from durable Task, Workspace, repository, and plugin state. The Kubernetes
//! pod watcher also uses the server-config/disabled-server parsing helpers when
//! classifying and remediating server failures. HTTP and adapter callers do not
//! provision through this module; they call `state.sandbox`.

use tracing::{info, warn};

use moatless_backend_core::scope::ScopeContext;

use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::plugin::activation;
use crate::plugin::dao::PluginRepository;
use crate::repository::dao::RepoRepository;
use crate::repository::models::RepositoryProvider;
use crate::sandbox::error::SandboxError;
use crate::sandbox::models::{ProvisionRequest, RepoConfig, ServerConfig};
use moatless_api_schemas::plugin::PluginDelivery;
use crate::task::dao::TaskRepository;
use crate::task::models::Task;
use crate::workspace::dao::WorkspaceRepository;

// Re-export the workspace-sourced config builders from k8s/auto_provision. They
// are shared by lifecycle plan construction and pod-watcher parsing.
pub use crate::sandbox::k8s::auto_provision::{
    build_repo_config_from_workspace, build_server_configs_from_workspace, filter_disabled_servers,
    parse_disabled_servers, parse_env_vars_from_workspace,
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
    plugins: &dyn PluginRepository,
    workspaces: &dyn WorkspaceRepository,
    task_id: &str,
) -> Result<ProvisionRequest, SandboxError> {
    // 1. Read task from DB
    // reason: lifecycle plan construction acts on behalf of the system, not a user
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let task = tasks
        .find_by_id(task_id, &sys)
        .await
        .map_err(|source| SandboxError::Database {
            context: "reading the task row for provisioning",
            source,
        })?
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

    // 2. Read repository from DB. A missing row is a permanent misconfiguration
    // (the task references a repository that does not exist) — retrying can't
    // create it — so it surfaces as a client error, not a transient one. A DB
    // read error stays `Internal` (transient).
    // reason: provision plan reads any repository regardless of owner
    #[allow(clippy::disallowed_methods)]
    let repo_sys = ScopeContext::system();
    let repo = repos
        .find_by_id(&repository_id, &repo_sys)
        .await
        .map_err(|source| SandboxError::Database {
            context: "reading the repository row for provisioning",
            source,
        })?
        .ok_or_else(|| {
            SandboxError::BadRequest(format!(
                "Repository '{repository_id}' not found in database"
            ))
        })?;

    let git_provider = repo.provider.unwrap_or(RepositoryProvider::Github);

    let git_clone_url = repo.remote_url.clone().unwrap_or_default();
    let git_branch = task.branch.clone().or(repo.default_branch.clone());
    let user_id = task.created_by.as_deref().unwrap_or("system").to_string();

    // Run-config source: the task's workspace. insert_task backfills a shadow
    // workspace for every task, so this is present for any real task. The repo
    // row is still read above for git_clone_url/branch/provider (primary-repo
    // checkout only this iteration).
    let workspace = match task.workspace_id.as_deref() {
        Some(ws_id) => workspaces
            .find_by_id(ws_id, &repo_sys)
            .await
            .map_err(|source| SandboxError::Database {
                context: "reading the workspace row for provisioning",
                source,
            })?,
        None => None,
    };

    let (mut repo_config, mut env_vars) = if let Some(ws) = &workspace {
        (
            build_repo_config_from_workspace(ws),
            parse_env_vars_from_workspace(ws),
        )
    } else {
        warn!(
            task_id = %task_id,
            workspace_id = ?task.workspace_id,
            "Auto-provision: task has no workspace; provisioning with git-connection only"
        );
        (minimal_repo_config(&repo.name), None)
    };

    // Select the tmux Claude transport per the catalog when the task's agent
    // requests it. Each task is its own pod, so the pod-level flag is exactly
    // the right granularity.
    if crate::agent::catalog::use_tmux_for(task.agent_type.0.as_str()) {
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("SANDBOX_USE_TMUX".to_string(), "true".to_string());
    }

    // The Task's selected Agent harness is fixed for the Sandbox lifetime
    // (ADR-0004): pass it at construction so the sandbox resolves its harness
    // once and pending Messages can no longer switch it. Inserted after the
    // workspace env so the authoritative Task selection overrides any stray
    // workspace-level `AGENT_TYPE`.
    env_vars
        .get_or_insert_with(std::collections::HashMap::new)
        .insert("AGENT_TYPE".to_string(), task.agent_type.0.clone());

    // Resolve the plugins this task's owner has turned on. A task with no
    // identifiable owner gets the global defaults and nothing personal — both
    // columns are polymorphic text, so a sentinel like `system` or a `bot:`
    // value is nobody rather than an error.
    let owner = task
        .owner_user_id
        .as_deref()
        .or(task.created_by.as_deref())
        .and_then(|id| uuid::Uuid::parse_str(id).ok());
    match resolve_plugin_deliveries(plugins, owner).await {
        Ok(deliveries) if !deliveries.is_empty() => {
            info!(
                task_id = %task_id,
                count = deliveries.len(),
                "Auto-provision: resolved plugins"
            );
            repo_config.plugins = Some(deliveries);
        }
        Ok(_) => {}
        Err(e) => {
            // A task that starts without its plugins is worse than one that
            // does not start at all only in theory; in practice the agent still
            // works, so this stays non-fatal as it always has.
            warn!(
                task_id = %task_id,
                error = %e,
                "Auto-provision: failed to resolve plugins (non-fatal)"
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

    // Server overrides are deliberately absent from the plan. They are files on
    // the workspace volume, so a rebuilt pod finds them already in place; there
    // is no copy here to replay.

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

/// What a task's owner gets: their effective plugins, each with the skills the
/// sandbox is to keep.
async fn resolve_plugin_deliveries(
    plugins: &dyn PluginRepository,
    owner_user_id: Option<uuid::Uuid>,
) -> Result<Vec<PluginDelivery>, sqlx::Error> {
    Ok(activation::resolve_for_user(plugins, owner_user_id)
        .await?
        .into_iter()
        .map(|(plugin, skill_names)| PluginDelivery {
            name: plugin.name,
            git_url: plugin.git_url,
            skill_names,
        })
        .collect())
}

/// A run-config with only the repo name — used when a task unexpectedly has no
/// workspace. Provisioning then falls back to global defaults (no servers).
fn minimal_repo_config(name: &str) -> RepoConfig {
    RepoConfig {
        name: name.to_string(),
        docker_image: None,
        servers: None,
        system_prompt: None,
        code_execution: None,
        preview_config: None,
        resources: None,
        plugins: None,
        docker_access: None,
        docker_networks: None,
        sandbox_service_account: None,
        network_profile: None,
    }
}

/// Get server configs for a task from its workspace, returning an empty vec on
/// any failure.
///
/// Convenience wrapper that handles the Option<Arc<dyn Repo>> pattern
/// used throughout handlers.
pub async fn get_server_configs_for_task(
    tasks: &dyn TaskRepository,
    workspaces: &dyn WorkspaceRepository,
    task_id: &str,
) -> Vec<ServerConfig> {
    match resolve_server_configs(tasks, workspaces, task_id).await {
        Ok(configs) => configs,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to get server configs");
            vec![]
        }
    }
}

/// Resolve a task's server configs from its workspace's `servers` JSONB.
///
/// Resolves task_id → task → workspace → `build_server_configs_from_workspace`.
/// Returns `Err` on DB failures or missing rows so callers can log.
async fn resolve_server_configs(
    tasks: &dyn TaskRepository,
    workspaces: &dyn WorkspaceRepository,
    task_id: &str,
) -> Result<Vec<ServerConfig>, String> {
    // reason: server config resolution reads across all task owners for pod watcher
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let task = tasks
        .find_by_id(task_id, &sys)
        .await
        .map_err(|e| format!("Failed to fetch task {task_id}: {e}"))?
        .ok_or_else(|| format!("Task {task_id} not found"))?;
    let Some(ws_id) = task.workspace_id.as_deref() else {
        return Ok(vec![]);
    };
    let ws = workspaces
        .find_by_id(ws_id, &sys)
        .await
        .map_err(|e| format!("Failed to fetch workspace {ws_id}: {e}"))?
        .ok_or_else(|| format!("Workspace {ws_id} not found"))?;
    Ok(build_server_configs_from_workspace(&ws))
}
