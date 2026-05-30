//! Task credential resolution for sandbox runtime consumers.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::warn;

use crate::auth::credentials::resolve_user_github_token;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::auth::users::UserRepository;
use crate::repository::dao::RepoRepository;
use crate::repository::handlers::parse_github_owner_repo;
use crate::secret::models::is_reserved_key;
use crate::secret::service::SecretService;
use crate::task::dao::TaskRepository;

pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";
pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";
pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";
pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
pub const GIT_USER_NAME: &str = "GIT_USER_NAME";
pub const GIT_USER_EMAIL: &str = "GIT_USER_EMAIL";

pub struct TaskCredentialInputs<'a> {
    pub task_id: &'a str,
    pub user_id: &'a str,
    pub team_id: Option<&'a str>,
    pub secrets: Option<&'a Arc<SecretService>>,
    pub provider_tokens: Option<&'a Arc<dyn ProviderTokenRepository>>,
    pub github_app_service: Option<&'a Arc<GitHubAppService>>,
    pub provider_token_auth: Option<&'a Arc<dyn ProviderTokenAuthRepository>>,
    /// Registry consulted when the effective user's `provider_tokens` row has
    /// `auth_method = 'github_app_installation'`.
    pub github_apps: Option<&'a Arc<GitHubAppRegistry>>,
    /// Repo owner (e.g. `"acme"`) for the task's repository. Required for
    /// installation-token minting because installations are scoped per-org.
    pub repo_owner: Option<&'a str>,
    /// User repository — read to source `GIT_USER_NAME` / `GIT_USER_EMAIL`
    /// from `users.login` / `users.email` for bot identities.
    pub users: Option<&'a Arc<dyn UserRepository>>,
}

/// Resolve the credentials a task's sandbox needs.
///
/// Priority order (most-specific wins):
/// 1. Per-user / team `provider_tokens` (OAuth-managed, e.g. a freshly-issued
///    Codex `auth.json` from the device-code flow or a connected GitHub App
///    user-token).
/// 2. Scoped secrets — the `SecretService` already resolves these with
///    user > team > global precedence internally; we only fill keys that
///    aren't already covered by a `provider_token`.
///
/// Rationale: `provider_tokens` is the live OAuth/connection state and is
/// inherently per-user; a generic global secret with the same key (e.g. a
/// fallback `CODEX_AUTH_JSON` set by an admin) must NOT shadow it, otherwise
/// users who explicitly reconnect their account see no effect. Globals
/// remain a useful bottom-most fallback for users who haven't connected.
pub async fn resolve_task_credentials(input: TaskCredentialInputs<'_>) -> HashMap<String, String> {
    let mut credentials: HashMap<String, String> = HashMap::new();

    fill_provider_tokens(
        &mut credentials,
        input.provider_tokens,
        input.team_id,
        input.user_id,
        input.github_app_service,
        input.provider_token_auth,
        input.github_apps,
        input.repo_owner,
    )
    .await;

    fill_bot_identity(&mut credentials, input.users, input.user_id).await;

    let scoped =
        resolve_scoped_secrets(input.secrets, input.user_id, input.team_id, input.task_id).await;

    fill_missing_from_scoped(&mut credentials, scoped);
    credentials
}

/// Resolve only the GitHub token a task's sandbox would receive.
///
/// Mirrors the priority chain used by `resolve_task_credentials` (team
/// `provider_tokens` → user `provider_tokens` with GitHub-App refresh) but
/// skips the scoped-secret fallback: an operator-set `GITHUB_TOKEN` secret
/// is an escape hatch that we deliberately don't pre-validate at provision
/// time, so callers who use it can still start a sandbox even if the GitHub
/// API is unreachable from the backend.
pub async fn resolve_task_github_token(input: TaskCredentialInputs<'_>) -> Option<String> {
    let provider_tokens = input.provider_tokens?;
    let mut credentials: HashMap<String, String> = HashMap::new();

    if let Some(team_id) = input.team_id {
        fill_team_provider_tokens(&mut credentials, provider_tokens, team_id).await;
    }
    if !credentials.contains_key(GITHUB_TOKEN) {
        fill_user_provider_tokens(
            &mut credentials,
            provider_tokens,
            input.user_id,
            input.github_app_service,
            input.provider_token_auth,
            input.github_apps,
            input.repo_owner,
        )
        .await;
    }

    credentials.remove(GITHUB_TOKEN)
}

/// Merge scoped-secret values into `credentials` only for keys that aren't
/// already covered by a provider_token. Extracted so the priority logic is
/// directly unit-testable without standing up the full `SecretService`.
fn fill_missing_from_scoped(
    credentials: &mut HashMap<String, String>,
    scoped: HashMap<String, String>,
) {
    for (key, value) in scoped {
        credentials.entry(key).or_insert(value);
    }
}

/// Resolve the shared env delivered to every server sidecar.
///
/// Returns the full accessible scope chain (global > team > user) with
/// reserved runtime keys filtered out. The agent env is a superset that
/// additionally includes provider tokens.
pub async fn resolve_server_env(
    user_id: &str,
    team_id: Option<&str>,
    secrets: Option<&Arc<SecretService>>,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };
    match secrets.resolve_accessible_plaintext(user_id, team_id).await {
        Ok(mut env) => {
            env.retain(|key, _| !is_reserved_key(key));
            env
        }
        Err(e) => {
            warn!(user_id, error = %e, "Failed to resolve server env secrets");
            HashMap::new()
        }
    }
}

/// Resolve the GitHub `owner` (org or user namespace) for a task's repository.
/// Returns `None` when the task or repository isn't found, the repo has no
/// `remote_url`, or the URL doesn't parse as a GitHub path. Used to mint
/// per-org installation tokens for bot-user credentials.
pub(crate) async fn resolve_task_repo_owner(
    tasks: Option<&Arc<dyn TaskRepository>>,
    repositories: Option<&Arc<dyn RepoRepository>>,
    task_id: &str,
) -> Option<String> {
    let tasks = tasks?;
    let repositories = repositories?;
    let task = tasks.find_by_id_internal(task_id).await.ok().flatten()?;
    let repo = repositories
        .find_repository_by_id(&task.repository_id)
        .await
        .ok()
        .flatten()?;
    let remote_url = repo.remote_url.as_deref()?;
    parse_github_owner_repo(remote_url).map(|(owner, _)| owner)
}

pub(crate) async fn resolve_task_team_id(
    tasks: Option<&Arc<dyn TaskRepository>>,
    task_id: &str,
) -> Option<String> {
    let tasks = tasks?;
    match tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) => task.team_id,
        Ok(None) => {
            warn!(task_id, "resolve_task_team_id: task not found");
            None
        }
        Err(e) => {
            warn!(task_id, error = %e, "resolve_task_team_id: DB error");
            None
        }
    }
}

async fn resolve_scoped_secrets(
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
    team_id: Option<&str>,
    task_id: &str,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };

    match secrets.resolve_accessible_plaintext(user_id, team_id).await {
        Ok(credentials) => credentials,
        Err(e) => {
            warn!(task_id, error = %e, "Failed to resolve scoped secrets");
            HashMap::new()
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn fill_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: Option<&Arc<dyn ProviderTokenRepository>>,
    team_id: Option<&str>,
    user_id: &str,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthRepository>>,
    github_apps: Option<&Arc<GitHubAppRegistry>>,
    repo_owner: Option<&str>,
) {
    let Some(provider_tokens) = provider_tokens else {
        return;
    };

    if let Some(team_id) = team_id {
        fill_team_provider_tokens(credentials, provider_tokens, team_id).await;
    }
    fill_user_provider_tokens(
        credentials,
        provider_tokens,
        user_id,
        github_app_service,
        provider_token_auth,
        github_apps,
        repo_owner,
    )
    .await;
}

async fn fill_team_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenRepository>,
    team_id: &str,
) {
    for (provider, cred_key) in [
        ("github", GITHUB_TOKEN),
        ("gitlab", GITLAB_TOKEN),
        ("gitness", GITNESS_TOKEN),
        ("codex", CODEX_AUTH_JSON),
        ("openai", OPENAI_API_KEY),
    ] {
        if credentials.contains_key(cred_key) {
            continue;
        }
        match provider_tokens.find_team_token(team_id, provider).await {
            Ok(Some(row)) => {
                credentials.insert(cred_key.to_string(), row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, team_id, provider, "Failed to query team provider_token");
            }
        }
    }

    if credentials.contains_key(CLAUDE_CODE_OAUTH_TOKEN)
        || credentials.contains_key(ANTHROPIC_API_KEY)
    {
        return;
    }
    match provider_tokens.find_team_token(team_id, "claude").await {
        Ok(Some(row)) => {
            credentials.insert(CLAUDE_CODE_OAUTH_TOKEN.to_string(), row.access_token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, team_id, "Failed to query team claude provider_token");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn fill_user_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenRepository>,
    user_id: &str,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthRepository>>,
    github_apps: Option<&Arc<GitHubAppRegistry>>,
    repo_owner: Option<&str>,
) {
    if !credentials.contains_key(GITHUB_TOKEN) {
        if let Some(registry) = github_apps {
            match resolve_user_github_token(
                provider_tokens.as_ref(),
                provider_token_auth.map(Arc::as_ref),
                github_app_service.map(Arc::as_ref),
                registry.as_ref(),
                user_id,
                repo_owner,
            )
            .await
            {
                Ok(Some(token)) => {
                    credentials.insert(GITHUB_TOKEN.to_string(), token);
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(error = %e, user_id, "Failed to resolve GitHub token for sandbox");
                }
            }
        } else if let Ok(Some(row)) =
            provider_tokens.find_provider_token(user_id, "github").await
        {
            // Registry not wired (legacy code paths without GitHubAppRegistry):
            // fall back to the raw token. We can't refresh or mint here.
            credentials.insert(GITHUB_TOKEN.to_string(), row.access_token);
        }
    }

    for (provider, cred_key) in [
        ("gitlab", GITLAB_TOKEN),
        ("gitness", GITNESS_TOKEN),
        ("codex", CODEX_AUTH_JSON),
        ("openai", OPENAI_API_KEY),
    ] {
        if credentials.contains_key(cred_key) {
            continue;
        }
        match provider_tokens.find_provider_token(user_id, provider).await {
            Ok(Some(row)) => {
                credentials.insert(cred_key.to_string(), row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, user_id, provider, "Failed to query provider_token");
            }
        }
    }

    if credentials.contains_key(CLAUDE_CODE_OAUTH_TOKEN)
        || credentials.contains_key(ANTHROPIC_API_KEY)
    {
        return;
    }
    match provider_tokens.find_provider_token(user_id, "claude").await {
        Ok(Some(row)) => {
            credentials.insert(CLAUDE_CODE_OAUTH_TOKEN.to_string(), row.access_token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, user_id, "Failed to query claude provider_token");
        }
    }
}

/// When the credentialed user is a bot, source git author identity from the
/// `users` row so commits and pushes attribute to `{slug}[bot]` instead of
/// whatever `gh api user` would resolve from the installation token (which
/// has no associated user).
async fn fill_bot_identity(
    credentials: &mut HashMap<String, String>,
    users: Option<&Arc<dyn UserRepository>>,
    user_id: &str,
) {
    let Some(users) = users else { return };
    let user = match users.get_user(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return,
        Err(e) => {
            warn!(error = %e, user_id, "Failed to look up user for bot identity");
            return;
        }
    };
    if !user.is_bot {
        return;
    }
    credentials.insert(GIT_USER_NAME.to_string(), user.login);
    if let Some(email) = user.email {
        credentials.insert(GIT_USER_EMAIL.to_string(), email);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_token_value_wins_over_scoped_secret_with_same_key() {
        // Repro of the global-secret-shadowing bug: a stale global secret with
        // key CODEX_AUTH_JSON must NOT override a freshly-rotated per-user
        // provider_token under the same key.
        let mut credentials = HashMap::new();
        credentials.insert(
            CODEX_AUTH_JSON.to_string(),
            "fresh-from-provider-token".to_string(),
        );

        let mut scoped = HashMap::new();
        scoped.insert(
            CODEX_AUTH_JSON.to_string(),
            "stale-global-secret-april-9".to_string(),
        );
        scoped.insert("FOO".to_string(), "bar".to_string());

        fill_missing_from_scoped(&mut credentials, scoped);

        assert_eq!(
            credentials[CODEX_AUTH_JSON], "fresh-from-provider-token",
            "provider_token must beat scoped secret with same key"
        );
        // Unique scoped keys still flow through.
        assert_eq!(credentials["FOO"], "bar");
    }

    #[test]
    fn scoped_secret_fills_in_when_no_provider_token() {
        // Inverse: with no provider_token populated, the scoped secret should
        // still serve as a usable fallback (e.g. a global CODEX_AUTH_JSON for
        // users who haven't connected via OAuth).
        let mut credentials = HashMap::new();

        let mut scoped = HashMap::new();
        scoped.insert(CODEX_AUTH_JSON.to_string(), "global-fallback".to_string());

        fill_missing_from_scoped(&mut credentials, scoped);

        assert_eq!(credentials[CODEX_AUTH_JSON], "global-fallback");
    }

    #[test]
    fn fill_missing_does_not_drop_unique_provider_keys() {
        // Keys that appear ONLY on the provider_token side must remain.
        let mut credentials = HashMap::new();
        credentials.insert(GITHUB_TOKEN.to_string(), "ghs_x".to_string());

        let scoped: HashMap<String, String> = HashMap::new();

        fill_missing_from_scoped(&mut credentials, scoped);

        assert_eq!(credentials[GITHUB_TOKEN], "ghs_x");
    }
}
