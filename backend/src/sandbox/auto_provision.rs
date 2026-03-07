//! Provider-agnostic auto-provisioning logic.
//!
//! Reads task + repository from DB, resolves credentials, and builds
//! a `ProvisionRequest`. Used by handlers and other modules that need
//! to ensure a sandbox is running without knowing provisioning details.

use std::env;
use tracing::{info, warn};

use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::repository::dao::RepoRepository;
use crate::repository::models::RepositoryProvider;
use crate::sandbox::error::SandboxError;
use crate::sandbox::models::{ProvisionRequest, ServerConfig};
use crate::task::dao::TaskRepository;

// Re-export the repo-config builders from k8s/auto_provision so existing
// callers can use the same path.
pub use crate::sandbox::k8s::auto_provision::{
    build_repo_config, build_server_configs, get_server_configs,
};

/// Build a `ProvisionRequest` from database state + resolved credentials.
///
/// Provider-agnostic: works for both K8s and Docker sandboxes.
///
/// Steps:
/// 1. Read the task row to get repository_id + branch
/// 2. Read the repository row to get repo config
/// 3. Resolve credentials (provider_tokens → env var fallback)
/// 4. Assemble the ProvisionRequest
pub async fn build_provision_request(
    tasks: &dyn TaskRepository,
    repos: &dyn RepoRepository,
    provider_tokens: &dyn ProviderTokenRepository,
    task_id: &str,
) -> Result<ProvisionRequest, SandboxError> {
    // 1. Read task from DB
    let task = tasks
        .find_by_id_internal(task_id)
        .await
        .map_err(|e| SandboxError::Internal(format!("DB error reading task: {e}")))?
        .ok_or_else(|| {
            SandboxError::Internal(format!("Task '{task_id}' not found in database"))
        })?;

    info!(
        task_id = %task_id,
        repository_id = %task.repository_id,
        "Auto-provision: task found in DB"
    );

    // 2. Read repository from DB
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

    // 3. Resolve credentials
    let git_provider = repo.provider.unwrap_or(RepositoryProvider::Github);
    let (git_provider_token, anthropic_api_key, claude_oauth_token) =
        resolve_credentials(task_id, provider_tokens, task.created_by.as_deref(), task.team_id.as_deref(), git_provider).await;

    // Ensure we have at least some auth
    if anthropic_api_key.is_none() && claude_oauth_token.is_none() {
        warn!(
            task_id = %task_id,
            "Auto-provision: no Anthropic API key or Claude OAuth token available"
        );
    }

    // 4. Build the ProvisionRequest
    let git_clone_url = repo.remote_url.clone().unwrap_or_default();
    let git_branch = task.branch.or(repo.default_branch.clone());

    let repo_config = build_repo_config(&repo);
    let env_vars = crate::sandbox::k8s::auto_provision::parse_env_vars(&repo);

    let plan = crate::sandbox::models::ProvisionPlan {
        repo_config,
        git_clone_url,
        git_branch,
        git_provider: git_provider.as_str().to_string(),
        git_provider_token,
        claude_oauth_token,
        anthropic_api_key,
        env_vars,
    };

    let user_id = task.created_by.unwrap_or_else(|| "system".to_string());

    Ok(ProvisionRequest { plan, user_id })
}

/// Resolve credentials with fallback chain:
/// 1. Query user-level `provider_tokens`
/// 2. Query team-level `provider_tokens` (if task has team_id)
/// 3. Fall back to environment variables
async fn resolve_credentials(
    task_id: &str,
    provider_tokens: &dyn ProviderTokenRepository,
    user_id: Option<&str>,
    team_id: Option<&str>,
    git_provider: RepositoryProvider,
) -> (Option<String>, Option<String>, Option<String>) {
    let mut git_token: Option<String> = None;
    let mut anthropic: Option<String> = None;
    let mut claude: Option<String> = None;

    let provider_str = git_provider.as_str();

    // Step 1: Fill from user provider_tokens table
    if let Some(uid) = user_id {
        match provider_tokens.find_provider_token(uid, provider_str).await {
            Ok(Some(row)) => {
                info!(task_id = %task_id, provider = %provider_str, "Auto-provision: git token resolved from user provider_tokens");
                git_token = Some(row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(task_id = %task_id, provider = %provider_str, error = %e, "Auto-provision: failed to query user provider_tokens for git provider");
            }
        }

        match provider_tokens.find_provider_token(uid, "claude").await {
            Ok(Some(row)) => {
                info!(task_id = %task_id, "Auto-provision: Claude token resolved from user provider_tokens");
                claude = Some(row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Auto-provision: failed to query user provider_tokens for claude");
            }
        }
    }

    // Step 2: Fill gaps from team provider_tokens
    if let Some(tid) = team_id {
        if git_token.is_none() {
            match provider_tokens.find_team_token(tid, provider_str).await {
                Ok(Some(row)) => {
                    info!(task_id = %task_id, provider = %provider_str, "Auto-provision: git token resolved from team provider_tokens");
                    git_token = Some(row.access_token);
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(task_id = %task_id, provider = %provider_str, error = %e, "Auto-provision: failed to query team provider_tokens for git provider");
                }
            }
        }

        if claude.is_none() {
            match provider_tokens.find_team_token(tid, "claude").await {
                Ok(Some(row)) => {
                    info!(task_id = %task_id, "Auto-provision: Claude token resolved from team provider_tokens");
                    claude = Some(row.access_token);
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Auto-provision: failed to query team provider_tokens for claude");
                }
            }
        }
    }

    // Step 3: Fill remaining gaps from environment variables (per-provider fallback)
    if git_token.is_none() {
        git_token = match git_provider {
            RepositoryProvider::Github => env::var("GITHUB_PAT").ok(),
            RepositoryProvider::Gitlab => env::var("GITLAB_PAT").ok(),
            RepositoryProvider::Gitness => env::var("GITNESS_PAT").ok(),
        };
    }
    if anthropic.is_none() {
        anthropic = env::var("ANTHROPIC_API_KEY").ok();
    }
    if claude.is_none() {
        claude = env::var("CLAUDE_CODE_OAUTH_TOKEN").ok();
    }

    info!(
        task_id = %task_id,
        provider = %provider_str,
        has_git_token = git_token.is_some(),
        has_anthropic = anthropic.is_some(),
        has_claude_oauth = claude.is_some(),
        "Auto-provision: credential resolution complete"
    );

    (git_token, anthropic, claude)
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
