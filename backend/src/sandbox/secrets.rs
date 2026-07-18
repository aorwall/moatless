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
use crate::credentials::CredentialService;
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
    /// from `users.login` / `users.email`. Bots get both; human users get
    /// their account email when set (name still resolves via the GitHub API).
    pub users: Option<&'a Arc<dyn UserRepository>>,
    /// Credential service used to refresh codex tokens just-in-time before
    /// handing the blob to the sandbox. When `None`, the codex branch falls
    /// back to a raw provider_tokens read.
    pub credentials_svc: Option<&'a Arc<CredentialService>>,
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
        input.credentials_svc,
        Some(input.task_id),
    )
    .await;

    fill_git_identity(&mut credentials, input.users, input.user_id).await;

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

/// Resolve the SSH private keys a task's sandbox should materialize into
/// `~/.ssh`.
///
/// These are `ssh_key`-kind scoped secrets, resolved with the same
/// `user > team > global` precedence as other secrets but delivered on a
/// dedicated channel so they are never exposed as environment variables to the
/// agent. Returns a `secret key name → private key` map.
pub async fn resolve_task_ssh_keys(
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
    team_id: Option<&str>,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };
    match secrets.resolve_accessible_ssh_keys(user_id, team_id).await {
        Ok(keys) => keys,
        Err(e) => {
            warn!(user_id, error = %e, "Failed to resolve ssh key secrets");
            HashMap::new()
        }
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
    credentials_svc: Option<&Arc<CredentialService>>,
    current_task_id: Option<&str>,
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
    fill_codex(
        credentials,
        provider_tokens,
        credentials_svc,
        user_id,
        team_id,
        current_task_id,
    )
    .await;
}

/// Resolve `CODEX_AUTH_JSON` for this task.
///
/// Delegates to [`CredentialService::ensure_fresh_codex_for`] which owns
/// the `user > team > global` precedence walk; this function is a thin
/// fallback shim for callers without the credential service wired (tests /
/// no-DB) — there we walk the same chain via raw DAO reads with no refresh.
async fn fill_codex(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenRepository>,
    credentials_svc: Option<&Arc<CredentialService>>,
    user_id: &str,
    team_id: Option<&str>,
    current_task_id: Option<&str>,
) {
    if credentials.contains_key(CODEX_AUTH_JSON) {
        return;
    }

    if let Some(svc) = credentials_svc {
        match svc
            .ensure_fresh_codex_for(user_id, team_id, current_task_id)
            .await
        {
            Ok(blob) => {
                credentials.insert(CODEX_AUTH_JSON.to_string(), blob);
            }
            Err(crate::credentials::CredentialError::NotFound(_)) => {
                // No codex row at any scope — leave the slot empty and let
                // the scoped-secret fallback try the env-var blob.
            }
            Err(e) => {
                warn!(error = %e, user_id, "ensure_fresh_codex_for failed");
            }
        }
        return;
    }

    // No CredentialService: walk the same precedence with raw DAO reads.
    use crate::credentials::codex::scope::CodexScope;
    let mut scopes: Vec<CodexScope> = Vec::with_capacity(3);
    scopes.push(CodexScope::user(user_id));
    if let Some(team_id) = team_id {
        scopes.push(CodexScope::team(team_id));
    }
    scopes.push(CodexScope::Global);

    for scope in scopes {
        match provider_tokens.find_codex_blob(&scope).await {
            Ok(Some(row)) => {
                credentials.insert(CODEX_AUTH_JSON.to_string(), row.access_token);
                return;
            }
            Ok(None) => continue,
            Err(e) => {
                warn!(error = %e, scope = %scope, "Failed to query codex provider_token");
            }
        }
    }
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

    // Codex is resolved through `fill_codex_with_precedence` which walks
    // user > team > global in one pass; intentionally not handled here.

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
        } else if let Ok(Some(row)) = provider_tokens.find_provider_token(user_id, "github").await {
            // Registry not wired (legacy code paths without GitHubAppRegistry):
            // fall back to the raw token. We can't refresh or mint here.
            credentials.insert(GITHUB_TOKEN.to_string(), row.access_token);
        }
    }

    for (provider, cred_key) in [
        ("gitlab", GITLAB_TOKEN),
        ("gitness", GITNESS_TOKEN),
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

    // Codex is resolved through `fill_codex_with_precedence` which walks
    // user > team > global in one pass; intentionally not handled here.

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

/// Source the git author identity for the task's sandbox from the owning
/// `users` row.
///
/// - For **bot** users, both name and email come from the row so commits and
///   pushes attribute to `{slug}[bot]` instead of whatever `gh api user` would
///   resolve from the installation token (which has no associated user).
/// - For **human** users, we override only the email (when the account has one
///   set) so commits use the user's real Moatless email instead of the
///   `{login}@users.noreply.github.com` fallback. The name is left to the
///   sandbox's GitHub-API resolution.
///
/// In both cases these land as `GIT_USER_NAME` / `GIT_USER_EMAIL` env vars in
/// the sandbox, which `configure_git` applies with highest precedence.
async fn fill_git_identity(
    credentials: &mut HashMap<String, String>,
    users: Option<&Arc<dyn UserRepository>>,
    user_id: &str,
) {
    let Some(users) = users else { return };
    let user = match users.get_user(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return,
        Err(e) => {
            warn!(error = %e, user_id, "Failed to look up user for git identity");
            return;
        }
    };
    if user.is_bot {
        credentials.insert(GIT_USER_NAME.to_string(), user.login);
        if let Some(email) = user.email {
            credentials.insert(GIT_USER_EMAIL.to_string(), email);
        }
        return;
    }
    // Human user: prefer their Moatless account name/email for commit
    // attribution when set; each falls back independently to the sandbox's
    // GitHub-API resolution when unset.
    if let Some(name) = user.name {
        credentials.insert(GIT_USER_NAME.to_string(), name);
    }
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

    fn make_user(
        login: &str,
        name: Option<&str>,
        email: Option<&str>,
        is_bot: bool,
    ) -> crate::auth::users::User {
        use chrono::{DateTime, Utc};
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("valid epoch");
        crate::auth::users::User {
            id: "u1".to_string(),
            login: login.to_string(),
            email: email.map(str::to_string),
            name: name.map(str::to_string),
            avatar_url: None,
            provider: "github".to_string(),
            role: crate::auth::models::UserRole::User,
            created_at: epoch,
            last_login_at: epoch,
            settings: serde_json::Value::Null,
            is_bot,
        }
    }

    fn users_returning(
        login: &'static str,
        name: Option<&'static str>,
        email: Option<&'static str>,
        is_bot: bool,
    ) -> Arc<dyn UserRepository> {
        use crate::auth::users::MockUserRepository;
        let mut mock = MockUserRepository::new();
        mock.expect_get_user()
            .returning(move |_| Ok(Some(make_user(login, name, email, is_bot))));
        Arc::new(mock)
    }

    #[tokio::test]
    async fn human_user_name_and_email_override() {
        // A regular (non-bot) user with both an account name and email should
        // have both flow through as GIT_USER_NAME / GIT_USER_EMAIL.
        let users = users_returning(
            "alice",
            Some("Alice Example"),
            Some("alice@example.com"),
            /* is_bot */ false,
        );

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert_eq!(
            credentials.get(GIT_USER_NAME).map(String::as_str),
            Some("Alice Example")
        );
        assert_eq!(
            credentials.get(GIT_USER_EMAIL).map(String::as_str),
            Some("alice@example.com")
        );
    }

    #[tokio::test]
    async fn human_user_email_only_defers_name() {
        // Email set but no name → only the email is overridden; the name is
        // left for the sandbox's GitHub-API resolution.
        let users = users_returning(
            "alice",
            None,
            Some("alice@example.com"),
            /* is_bot */ false,
        );

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert_eq!(
            credentials.get(GIT_USER_EMAIL).map(String::as_str),
            Some("alice@example.com")
        );
        assert!(
            !credentials.contains_key(GIT_USER_NAME),
            "human user name should defer to GitHub-API resolution when unset"
        );
    }

    #[tokio::test]
    async fn human_user_without_name_or_email_sets_nothing() {
        // No account name/email → no override; the sandbox falls back to the
        // GitHub API / noreply default entirely.
        let users = users_returning("alice", None, None, /* is_bot */ false);

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert!(!credentials.contains_key(GIT_USER_EMAIL));
        assert!(!credentials.contains_key(GIT_USER_NAME));
    }

    #[tokio::test]
    async fn bot_user_sets_login_name_and_email() {
        // Bots keep the existing behavior: name is the login, plus email.
        let users = users_returning(
            "acme[bot]",
            None,
            Some("acme[bot]@users.noreply.github.com"),
            /* is_bot */ true,
        );

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert_eq!(credentials.get(GIT_USER_NAME).map(String::as_str), Some("acme[bot]"));
        assert_eq!(
            credentials.get(GIT_USER_EMAIL).map(String::as_str),
            Some("acme[bot]@users.noreply.github.com")
        );
    }
}
