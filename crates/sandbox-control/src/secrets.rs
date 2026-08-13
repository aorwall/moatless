//! Task credential resolution for sandbox runtime consumers.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_core::secret_keys::{
    ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, CODEX_AUTH_JSON, GITHUB_TOKEN, OPENAI_API_KEY,
};
use tracing::warn;

use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_identity::auth::github::service::GitHubAppService;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::auth::provider_tokens::ProviderTokenAuthDao;
use moatless_identity::auth::users::UserDao;
use moatless_identity::secret::models::is_reserved_key;
use moatless_identity::secret::service::SecretService;
use moatless_tasks::credential_sync::CredentialSync;
use moatless_tasks::task::dao::TaskDao;
use moatless_workspaces::repository::access::resolve_repository_github_token_with;
use moatless_workspaces::repository::dao::RepositoryDao;
use moatless_workspaces::repository::models::Repository;
use moatless_workspaces::workspace::credentials::WorkspaceCredentialService;
use moatless_workspaces::workspace::error::WorkspaceError;

pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";
pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";
pub const GIT_USER_NAME: &str = "GIT_USER_NAME";
pub const GIT_USER_EMAIL: &str = "GIT_USER_EMAIL";

pub struct TaskCredentialInputs<'a> {
    pub task_id: &'a str,
    pub user_id: &'a str,
    pub secrets: Option<&'a Arc<SecretService>>,
    pub provider_tokens: Option<&'a Arc<dyn ProviderTokenDao>>,
    pub github_app_service: Option<&'a Arc<GitHubAppService>>,
    pub provider_token_auth: Option<&'a Arc<dyn ProviderTokenAuthDao>>,
    /// Registry consulted when the effective user's `provider_tokens` row has
    /// `auth_method = 'github_app_installation'`.
    pub github_apps: Option<&'a Arc<GitHubAppRegistry>>,
    /// Persisted repository — supplies the remote URL whose owner and name the
    /// run-as user's credential is minted against.
    pub repository: Option<&'a Repository>,
    /// User repository — read to source `GIT_USER_NAME` / `GIT_USER_EMAIL`
    /// from `users.login` / `users.email`. Bots get both; human users get
    /// their account email when set (name still resolves via the GitHub API).
    pub users: Option<&'a Arc<dyn UserDao>>,
    /// Codex credential orchestrator used to refresh codex tokens just-in-time
    /// before handing the blob to the sandbox. When `None`, the codex branch
    /// falls back to a raw provider_tokens read.
    pub credential_sync: Option<&'a Arc<CredentialSync>>,
    /// Provider credentials pinned to the task's Workspace, already decrypted
    /// and keyed by environment variable — resolve them with
    /// [`resolve_workspace_credentials`]. Highest precedence: every other
    /// source below fills only the slots these leave empty.
    pub workspace_credentials: HashMap<String, String>,
}

/// Resolve the credentials a task's sandbox needs.
///
/// Priority order (most-specific wins):
/// 1. Credentials pinned to the task's Workspace. A Workspace credential names
///    the account the Workspace runs as, so it outranks whoever started the
///    task; without that it would not be an override.
/// 2. Per-user `provider_tokens` (OAuth-managed, e.g. a freshly-issued
///    Codex `auth.json` from the device-code flow or a connected GitHub App
///    user-token).
/// 3. Scoped secrets — the `SecretService` already resolves these with
///    user > global precedence internally; we only fill keys that
///    aren't already covered by a `provider_token`.
///
/// Rationale: `provider_tokens` is the live OAuth/connection state and is
/// inherently per-user; a generic global secret with the same key (e.g. a
/// fallback `CODEX_AUTH_JSON` set by an admin) must NOT shadow it, otherwise
/// users who explicitly reconnect their account see no effect. Globals
/// remain a useful bottom-most fallback for users who haven't connected.
///
/// The order is enforced structurally rather than by a comparison: the map
/// starts as the workspace credentials and every `fill_*` below skips a key
/// that is already present, so a new source cannot silently outrank an older
/// one by being added in the wrong place.
pub async fn resolve_task_credentials(input: TaskCredentialInputs<'_>) -> HashMap<String, String> {
    let mut credentials: HashMap<String, String> = input.workspace_credentials;

    fill_provider_tokens(
        &mut credentials,
        input.provider_tokens,
        input.secrets,
        input.user_id,
        input.github_app_service,
        input.provider_token_auth,
        input.github_apps,
        input.repository,
        input.credential_sync,
        Some(input.task_id),
    )
    .await;

    fill_git_identity(&mut credentials, input.users, input.user_id).await;

    let scoped = resolve_scoped_secrets(input.secrets, input.user_id, input.task_id).await;

    fill_missing_from_scoped(&mut credentials, scoped);
    credentials
}

/// Resolve only the GitHub token a task's sandbox would receive.
///
/// Goes through the same run-as-user resolution as runtime credential
/// delivery, so preflight and the sandbox never disagree — including the
/// workspace override, which is checked first here for the same reason
/// [`resolve_task_credentials`] seeds its map with it.
pub async fn resolve_task_github_token(input: TaskCredentialInputs<'_>) -> Option<String> {
    if let Some(token) = input.workspace_credentials.get(GITHUB_TOKEN) {
        return Some(token.clone());
    }

    let provider_tokens = input.provider_tokens?;
    let mut credentials: HashMap<String, String> = HashMap::new();

    fill_github(
        &mut credentials,
        provider_tokens,
        input.secrets,
        input.user_id,
        input.github_app_service,
        input.provider_token_auth,
        input.github_apps,
        input.repository,
    )
    .await;

    if let Some(token) = credentials.remove(GITHUB_TOKEN) {
        return Some(token);
    }

    None
}

/// Merge scoped-secret values into `credentials` only for keys that aren't
/// already covered by a provider_token. Extracted so the priority logic is
/// directly unit-testable without standing up the full `SecretService`.
fn fill_missing_from_scoped(
    credentials: &mut HashMap<String, String>,
    scoped: HashMap<String, String>,
) {
    for (key, value) in scoped {
        if key == GITHUB_TOKEN {
            continue;
        }
        credentials.entry(key).or_insert(value);
    }
}

/// Resolve the SSH private keys a task's sandbox should materialize into
/// `~/.ssh`.
///
/// These are `ssh_key`-kind scoped secrets, resolved with the same
/// `user > global` precedence as other secrets but delivered on a
/// dedicated channel so they are never exposed as environment variables to the
/// agent. Returns a `secret key name → private key` map.
pub async fn resolve_task_ssh_keys(
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };
    match secrets.resolve_accessible_ssh_keys(user_id).await {
        Ok(keys) => keys,
        Err(e) => {
            warn!(user_id, error = %e, "Failed to resolve ssh key secrets");
            HashMap::new()
        }
    }
}

/// Resolve the shared env delivered to every server sidecar.
///
/// Returns the full accessible scope chain (user > global) with
/// reserved runtime keys filtered out. The agent env is a superset that
/// additionally includes provider tokens.
pub async fn resolve_server_env(
    user_id: &str,
    secrets: Option<&Arc<SecretService>>,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };
    match secrets.resolve_accessible_plaintext(user_id).await {
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

/// The two rows a task's credentials are resolved against: the repository whose
/// owner/name a GitHub credential is minted for, and the workspace whose pinned
/// credentials outrank everything else.
#[derive(Default)]
pub(crate) struct TaskCredentialTargets {
    pub repository: Option<Repository>,
    pub workspace_id: Option<String>,
}

/// Resolve the repository and workspace attached to a task, in one task read.
pub(crate) async fn resolve_task_credential_targets(
    tasks: Option<&Arc<dyn TaskDao>>,
    repositories: Option<&Arc<dyn RepositoryDao>>,
    task_id: &str,
) -> TaskCredentialTargets {
    let (Some(tasks), Some(repositories)) = (tasks, repositories) else {
        return TaskCredentialTargets::default();
    };
    // reason: repository lookup for sandbox provisioning is system-wide
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = moatless_backend_core::scope::ScopeContext::system();
    let Some(task) = tasks.find_by_id(task_id, &sys_ctx).await.ok().flatten() else {
        return TaskCredentialTargets::default();
    };
    TaskCredentialTargets {
        repository: repositories
            .find_by_id(&task.repository_id, &sys_ctx)
            .await
            .ok()
            .flatten(),
        workspace_id: task.workspace_id,
    }
}

/// Resolve the credentials pinned to a workspace, keyed by environment
/// variable.
///
/// A task with no workspace, or a caller with no credential service wired,
/// yields an empty map — the ordinary case, and the one every deployment
/// without a single workspace credential takes. A workspace that *does* hold
/// credentials whose values cannot be read is an error: falling back would run
/// the task under a different identity than it was configured with. See
/// [`WorkspaceCredentialService::resolve_env`].
pub async fn resolve_workspace_credentials(
    workspace_credentials: Option<&Arc<WorkspaceCredentialService>>,
    workspace_id: Option<&str>,
) -> Result<HashMap<String, String>, WorkspaceError> {
    let (Some(service), Some(workspace_id)) = (workspace_credentials, workspace_id) else {
        return Ok(HashMap::new());
    };
    service.resolve_env(workspace_id).await
}

async fn resolve_scoped_secrets(
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
    task_id: &str,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };

    match secrets.resolve_accessible_plaintext(user_id).await {
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
    provider_tokens: Option<&Arc<dyn ProviderTokenDao>>,
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthDao>>,
    github_apps: Option<&Arc<GitHubAppRegistry>>,
    repository: Option<&Repository>,
    credential_sync: Option<&Arc<CredentialSync>>,
    current_task_id: Option<&str>,
) {
    let Some(provider_tokens) = provider_tokens else {
        return;
    };

    fill_github(
        credentials,
        provider_tokens,
        secrets,
        user_id,
        github_app_service,
        provider_token_auth,
        github_apps,
        repository,
    )
    .await;
    fill_user_provider_tokens(credentials, provider_tokens, user_id).await;
    fill_codex(
        credentials,
        provider_tokens,
        credential_sync,
        user_id,
        current_task_id,
    )
    .await;
}

/// Resolve `GITHUB_TOKEN` for this task.
///
/// `user_id` is the task's run-as identity and is the sole credential
/// authority: a sandbox gets GitHub access only through that user's own row,
/// never the repository's or the global one. A user holding no row leaves the
/// slot empty, and preflight refuses to start the sandbox.
#[allow(clippy::too_many_arguments)]
// reason: mirrors `fill_user_provider_tokens`' dependency list; bundling would
// only move the argument count to the call site.
async fn fill_github(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenDao>,
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthDao>>,
    github_apps: Option<&Arc<GitHubAppRegistry>>,
    repository: Option<&Repository>,
) {
    if credentials.contains_key(GITHUB_TOKEN) {
        return;
    }

    // An empty registry stands in when none is wired: only bot rows need it,
    // and for those an error beats handing the sandbox the placeholder.
    let fallback_registry = GitHubAppRegistry::default();
    let registry = github_apps.map_or(&fallback_registry, Arc::as_ref);

    let Some(repository) = repository else {
        return;
    };
    let Some(remote_url) = repository.remote_url.as_deref() else {
        return;
    };

    match resolve_repository_github_token_with(
        provider_tokens.as_ref(),
        provider_token_auth.map(Arc::as_ref),
        github_app_service.map(Arc::as_ref),
        registry,
        secrets.map(Arc::as_ref),
        repository,
        remote_url,
        Some(user_id),
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
}

/// Resolve `CODEX_AUTH_JSON` for this task.
///
/// Delegates to [`CredentialSync::ensure_fresh_codex_for`] which owns
/// the `user > global` precedence walk; this function is a thin
/// fallback shim for callers without the orchestrator wired (tests /
/// no-DB) — there we walk the same chain via raw DAO reads with no refresh.
async fn fill_codex(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenDao>,
    credential_sync: Option<&Arc<CredentialSync>>,
    user_id: &str,
    current_task_id: Option<&str>,
) {
    if credentials.contains_key(CODEX_AUTH_JSON) {
        return;
    }

    if let Some(svc) = credential_sync {
        match svc.ensure_fresh_codex_for(user_id, current_task_id).await {
            Ok(blob) => {
                credentials.insert(CODEX_AUTH_JSON.to_string(), blob);
            }
            Err(moatless_workspaces::credentials::CredentialError::NotFound(_)) => {
                // No codex row at any scope — leave the slot empty and let
                // the scoped-secret fallback try the env-var blob.
            }
            Err(e) => {
                warn!(error = %e, user_id, "ensure_fresh_codex_for failed");
            }
        }
        return;
    }

    // No CredentialService: walk user > global with raw DAO reads.
    use moatless_identity::auth::codex_scope::CodexScope;
    let scopes = [CodexScope::user(user_id), CodexScope::Global];

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

async fn fill_user_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenDao>,
    user_id: &str,
) {
    // GitHub is resolved through `fill_github`; intentionally not handled here.
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

    // Codex is resolved through `fill_codex` which walks user > global in one
    // pass; intentionally not handled here.

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
    users: Option<&Arc<dyn UserDao>>,
    user_id: &str,
) {
    let Some(users) = users else { return };
    let Ok(uuid) = user_id.parse::<uuid::Uuid>() else {
        return;
    };
    let user = match users.get_user(&uuid).await {
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
    use moatless_identity::auth::provider_tokens::AUTH_METHOD_SECRET_REF;

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
    fn scoped_github_secret_is_never_injected() {
        let mut credentials = HashMap::new();
        let scoped = HashMap::from([(GITHUB_TOKEN.to_string(), "ghp_scope_bypass".to_string())]);

        fill_missing_from_scoped(&mut credentials, scoped);

        assert!(!credentials.contains_key(GITHUB_TOKEN));
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

    // --- preflight trust: which GITHUB_TOKEN sources satisfy provisioning ---

    use moatless_identity::auth::test_support::StubProviderTokens;
    use moatless_identity::secret::test_support::{secret, service_with};

    fn github_repository(scope: moatless_backend_core::scope::Scope) -> Repository {
        Repository {
            id: "repo-1".to_string(),
            name: "repo".to_string(),
            kind: moatless_workspaces::repository::models::RepositoryKind::Remote,
            remote_url: Some("https://github.com/acme/repo.git".to_string()),
            default_branch: None,
            description: None,
            provider: Some(moatless_workspaces::repository::models::RepositoryProvider::Github),
            scope,
            owner_user_id: Some("u1".to_string()),
            created_by: Some("u1".to_string()),
            access_status: "unchecked".to_string(),
            access_checked_at: None,
            access_error: None,
            deleted: Some(false),
            created_at: None,
            updated_at: None,
        }
    }

    /// Inputs with only the pieces `resolve_task_github_token` reads.
    fn github_inputs<'a>(
        provider_tokens: &'a Arc<dyn ProviderTokenDao>,
        secrets: Option<&'a Arc<SecretService>>,
        repository: &'a Repository,
    ) -> TaskCredentialInputs<'a> {
        workspace_github_inputs(provider_tokens, secrets, repository, HashMap::new())
    }

    /// The same, with credentials pinned to the task's workspace.
    fn workspace_github_inputs<'a>(
        provider_tokens: &'a Arc<dyn ProviderTokenDao>,
        secrets: Option<&'a Arc<SecretService>>,
        repository: &'a Repository,
        workspace_credentials: HashMap<String, String>,
    ) -> TaskCredentialInputs<'a> {
        TaskCredentialInputs {
            task_id: "t1",
            user_id: "u1",
            secrets,
            provider_tokens: Some(provider_tokens),
            github_app_service: None,
            provider_token_auth: None,
            github_apps: None,
            repository: Some(repository),
            users: None,
            credential_sync: None,
            workspace_credentials,
        }
    }

    // --- workspace credentials outrank every other source ---

    #[tokio::test]
    async fn a_workspace_credential_beats_the_run_as_user_row() {
        // The point of the feature: the workspace names the account, not
        // whoever happened to start the task.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row("oauth", "gho_from_user"),
        )));
        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let pinned = HashMap::from([(GITHUB_TOKEN.to_string(), "ghp_workspace".to_string())]);

        let token =
            resolve_task_github_token(workspace_github_inputs(&tokens, None, &repository, pinned))
                .await;
        assert_eq!(token, Some("ghp_workspace".to_string()));
    }

    #[tokio::test]
    async fn a_workspace_credential_supplies_the_token_when_the_user_holds_none() {
        // Without the override this user's task is refused by preflight
        // (`a_run_as_user_with_no_row_never_reaches_the_global_token`).
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(None));
        let repository = github_repository(moatless_backend_core::scope::Scope::Global);
        let pinned = HashMap::from([(GITHUB_TOKEN.to_string(), "ghp_workspace".to_string())]);

        let token =
            resolve_task_github_token(workspace_github_inputs(&tokens, None, &repository, pinned))
                .await;
        assert_eq!(token, Some("ghp_workspace".to_string()));
    }

    #[tokio::test]
    async fn a_workspace_credential_for_another_provider_leaves_github_alone() {
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row("oauth", "gho_from_user"),
        )));
        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let pinned = HashMap::from([(OPENAI_API_KEY.to_string(), "sk-workspace".to_string())]);

        let token =
            resolve_task_github_token(workspace_github_inputs(&tokens, None, &repository, pinned))
                .await;
        assert_eq!(token, Some("gho_from_user".to_string()));
    }

    #[tokio::test]
    async fn a_pinned_credential_survives_the_whole_resolution_chain() {
        // End to end through `resolve_task_credentials`: the workspace value
        // must beat the user's provider_token *and* the scoped secret that
        // would otherwise fill the same slot.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(None));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "env",
            OPENAI_API_KEY,
            "sk-from-secret",
        )]));
        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let pinned = HashMap::from([
            (GITHUB_TOKEN.to_string(), "ghp_workspace".to_string()),
            (OPENAI_API_KEY.to_string(), "sk-workspace".to_string()),
        ]);

        let credentials = resolve_task_credentials(workspace_github_inputs(
            &tokens,
            Some(&secrets),
            &repository,
            pinned,
        ))
        .await;

        assert_eq!(credentials[GITHUB_TOKEN], "ghp_workspace");
        assert_eq!(credentials[OPENAI_API_KEY], "sk-workspace");
    }

    #[tokio::test]
    async fn resolving_a_workspace_with_no_service_wired_yields_no_credentials() {
        assert!(
            resolve_workspace_credentials(None, Some("ws_1"))
                .await
                .expect("absent service is not an error")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn standalone_provider_token_secret_cannot_bypass_repository_scope() {
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(None));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_validated",
        )]));

        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let token =
            resolve_task_github_token(github_inputs(&tokens, Some(&secrets), &repository)).await;
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn env_kind_secret_cannot_bypass_repository_scope() {
        // A plain `env` GITHUB_TOKEN is never injected into a repository
        // sandbox, so preflight and runtime delivery cannot disagree.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(None));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "env",
            GITHUB_TOKEN,
            "ghp_unvalidated",
        )]));

        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let token =
            resolve_task_github_token(github_inputs(&tokens, Some(&secrets), &repository)).await;
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn provider_tokens_row_still_wins_over_the_secret() {
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row("oauth", "gho_from_row"),
        )));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_from_secret",
        )]));

        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let token =
            resolve_task_github_token(github_inputs(&tokens, Some(&secrets), &repository)).await;
        assert_eq!(token, Some("gho_from_row".to_string()));
    }

    #[tokio::test]
    async fn run_as_user_row_wins_over_a_global_scoped_repository() {
        // A trigger's run_as_user owns the credential: their own row is used
        // even though the repository is Global-scoped and a global binding
        // exists. Without this the sandbox silently ran as the global App.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(
            StubProviderTokens::with_user_row(Some(
                moatless_identity::auth::test_support::github_row("oauth", "gho_run_as_user"),
            ))
            .with_global_row(moatless_identity::auth::test_support::github_row(
                "oauth",
                "gho_global_binding",
            )),
        );

        let repository = github_repository(moatless_backend_core::scope::Scope::Global);
        let token = resolve_task_github_token(github_inputs(&tokens, None, &repository)).await;
        assert_eq!(token, Some("gho_run_as_user".to_string()));
    }

    #[tokio::test]
    async fn a_run_as_user_with_no_row_never_reaches_the_global_token() {
        // The sandbox's run-as user holds nothing, so the slot stays empty even
        // with a global row sitting there. Preflight turns this into a
        // NoGithubToken naming the user rather than starting the sandbox.
        let tokens: Arc<dyn ProviderTokenDao> =
            Arc::new(StubProviderTokens::with_user_row(None).with_global_row(
                moatless_identity::auth::test_support::github_row("oauth", "gho_global_binding"),
            ));

        let repository = github_repository(moatless_backend_core::scope::Scope::Global);
        let token = resolve_task_github_token(github_inputs(&tokens, None, &repository)).await;
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn secret_ref_row_resolves_via_the_secret_not_the_sentinel() {
        // No App registry wired: the fallback dispatch still has to honour
        // `secret_ref`, or the sandbox would receive the sentinel string.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row(
                AUTH_METHOD_SECRET_REF,
                moatless_identity::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER,
            ),
        )));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_from_secret",
        )]));

        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let token =
            resolve_task_github_token(github_inputs(&tokens, Some(&secrets), &repository)).await;
        assert_eq!(token, Some("ghp_from_secret".to_string()));
    }

    #[tokio::test]
    async fn secret_ref_row_without_a_secret_service_resolves_to_none() {
        // Pins the trap that made preflight report a connected PAT as "GitHub
        // account not connected": with no secret service the sentinel cannot be
        // dereferenced, and the error is swallowed into an absent credential.
        // Callers must wire `secrets` — see `preflight_validate_github_token`.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row(
                AUTH_METHOD_SECRET_REF,
                moatless_identity::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER,
            ),
        )));

        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let token = resolve_task_github_token(github_inputs(&tokens, None, &repository)).await;
        assert_eq!(token, None);
    }

    // --- GITHUB_TOKEN exact repository scope ---

    #[tokio::test]
    async fn global_github_token_does_not_backstop_user_scope() {
        let tokens: Arc<dyn ProviderTokenDao> =
            Arc::new(StubProviderTokens::default().with_global_row(
                moatless_identity::auth::test_support::github_row("manual", "ghp_org"),
            ));

        let repository = github_repository(moatless_backend_core::scope::Scope::User);
        let token = resolve_task_github_token(github_inputs(&tokens, None, &repository)).await;
        assert_eq!(token, None);
    }

    fn make_user(
        login: &str,
        name: Option<&str>,
        email: Option<&str>,
        is_bot: bool,
    ) -> moatless_identity::auth::users::User {
        use chrono::{DateTime, Utc};
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("valid epoch");
        moatless_identity::auth::users::User {
            id: uuid::Uuid::from_u128(1),
            provider_user_id: None,
            login: login.to_string(),
            email: email.map(str::to_string),
            name: name.map(str::to_string),
            avatar_url: None,
            provider: "github".to_string(),
            role: moatless_identity::auth::models::UserRole::User,
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
    ) -> Arc<dyn UserDao> {
        use moatless_identity::auth::users::MockUserDao;
        let mut mock = MockUserDao::new();
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
        fill_git_identity(
            &mut credentials,
            Some(&users),
            &uuid::Uuid::from_u128(1).to_string(),
        )
        .await;

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
        fill_git_identity(
            &mut credentials,
            Some(&users),
            &uuid::Uuid::from_u128(1).to_string(),
        )
        .await;

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
        fill_git_identity(
            &mut credentials,
            Some(&users),
            &uuid::Uuid::from_u128(1).to_string(),
        )
        .await;

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
        fill_git_identity(
            &mut credentials,
            Some(&users),
            &uuid::Uuid::from_u128(1).to_string(),
        )
        .await;

        assert_eq!(
            credentials.get(GIT_USER_NAME).map(String::as_str),
            Some("acme[bot]")
        );
        assert_eq!(
            credentials.get(GIT_USER_EMAIL).map(String::as_str),
            Some("acme[bot]@users.noreply.github.com")
        );
    }
}
