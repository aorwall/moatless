//! Task credential resolution for sandbox runtime consumers.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::warn;

use crate::auth::credentials::resolve_github_token_for_scopes;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::auth::users::UserRepository;
use crate::credentials::CredentialService;
use crate::repository::dao::RepoRepository;
use crate::repository::handlers::parse_github_owner_repo;
use crate::secret::models::{is_reserved_key, SecretKind};
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
        input.secrets,
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
/// Runs the same `user > team > global` walk as `resolve_task_credentials`,
/// so preflight and provisioning cannot disagree about whether a token exists.
///
/// `input.secrets` must be wired: it dereferences `secret_ref` rows, whose
/// `access_token` holds a sentinel rather than a token. Passing `None` makes
/// every PAT-backed credential resolve as absent.
///
/// Skips only the `env`-kind scoped-secret fallback: an operator-set
/// `GITHUB_TOKEN` env secret is an escape hatch that we deliberately don't
/// pre-validate at provision time, so callers who use it can still start a
/// sandbox even if the GitHub API is unreachable from the backend.
pub async fn resolve_task_github_token(input: TaskCredentialInputs<'_>) -> Option<String> {
    let provider_tokens = input.provider_tokens?;
    let mut credentials: HashMap<String, String> = HashMap::new();

    fill_github(
        &mut credentials,
        provider_tokens,
        input.secrets,
        input.user_id,
        input.team_id,
        input.github_app_service,
        input.provider_token_auth,
        input.github_apps,
        input.repo_owner,
    )
    .await;

    if let Some(token) = credentials.remove(GITHUB_TOKEN) {
        return Some(token);
    }

    // A `provider_token`-kind secret is a PAT this backend validated against
    // GitHub, so preflight can trust it the same way it trusts a
    // `provider_tokens` row. A plain `env` GITHUB_TOKEN deliberately does not
    // satisfy preflight — it stays the unvalidated operator escape hatch.
    resolve_validated_github_secret(input.secrets, input.user_id).await
}

/// Read a `provider_token`-kind `GITHUB_TOKEN` user secret. Errors are logged
/// and treated as absent — preflight failing open here would block sandbox
/// startup on a secrets-backend hiccup.
async fn resolve_validated_github_secret(
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
) -> Option<String> {
    let secrets = secrets?;
    match secrets
        .resolve_user_secret(user_id, GITHUB_TOKEN, Some(SecretKind::ProviderToken))
        .await
    {
        Ok(token) => token,
        Err(e) => {
            warn!(user_id, error = %e, "Failed to read validated GITHUB_TOKEN secret");
            None
        }
    }
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
/// Returns the full accessible scope chain (user > team > global) with
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
    secrets: Option<&Arc<SecretService>>,
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

    fill_github(
        credentials,
        provider_tokens,
        secrets,
        user_id,
        team_id,
        github_app_service,
        provider_token_auth,
        github_apps,
        repo_owner,
    )
    .await;
    if let Some(team_id) = team_id {
        fill_team_provider_tokens(credentials, provider_tokens, team_id).await;
    }
    fill_user_provider_tokens(credentials, provider_tokens, user_id).await;
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

/// Resolve `GITHUB_TOKEN` for this task.
///
/// Delegates to [`resolve_github_token_for_scopes`], which owns the
/// `user > team > global` precedence walk — the user's own credential wins,
/// and an admin-set org-wide token backstops users who have none. Routing
/// GitHub through the canonical resolver (rather than the raw team/user
/// `access_token` reads used for the other providers) is also what keeps a
/// `secret_ref` row from delivering its sentinel to the sandbox.
#[allow(clippy::too_many_arguments)]
// reason: mirrors `fill_user_provider_tokens`' dependency list; bundling would
// only move the argument count to the call site.
async fn fill_github(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenRepository>,
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
    team_id: Option<&str>,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthRepository>>,
    github_apps: Option<&Arc<GitHubAppRegistry>>,
    repo_owner: Option<&str>,
) {
    if credentials.contains_key(GITHUB_TOKEN) {
        return;
    }

    let team_ids: Vec<String> = team_id.map(str::to_string).into_iter().collect();
    // An empty registry stands in when none is wired: only bot rows need it,
    // and for those an error beats handing the sandbox the placeholder.
    let fallback_registry = GitHubAppRegistry::default();
    let registry = github_apps.map_or(&fallback_registry, Arc::as_ref);

    match resolve_github_token_for_scopes(
        provider_tokens.as_ref(),
        provider_token_auth.map(Arc::as_ref),
        github_app_service.map(Arc::as_ref),
        registry,
        secrets.map(Arc::as_ref),
        user_id,
        &team_ids,
        repo_owner,
    )
    .await
    {
        Ok(Some(resolved)) => {
            credentials.insert(GITHUB_TOKEN.to_string(), resolved.token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, user_id, "Failed to resolve GitHub token for sandbox");
        }
    }
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
    // GitHub is resolved through `fill_github`, which walks user > team >
    // global in one pass; intentionally not handled here. Reading the team
    // row directly would both shadow the user's own token and hand a
    // `secret_ref` row's sentinel to the sandbox.
    for (provider, cred_key) in [
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
    use crate::auth::provider_tokens::AUTH_METHOD_SECRET_REF;

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

    // --- preflight trust: which GITHUB_TOKEN sources satisfy provisioning ---

    use crate::auth::test_support::StubProviderTokens;
    use crate::secret::test_support::{secret, service_with};

    /// Inputs with only the pieces `resolve_task_github_token` reads.
    fn github_inputs<'a>(
        provider_tokens: &'a Arc<dyn ProviderTokenRepository>,
        secrets: Option<&'a Arc<SecretService>>,
    ) -> TaskCredentialInputs<'a> {
        TaskCredentialInputs {
            task_id: "t1",
            user_id: "u1",
            team_id: None,
            secrets,
            provider_tokens: Some(provider_tokens),
            github_app_service: None,
            provider_token_auth: None,
            github_apps: None,
            repo_owner: None,
            users: None,
            credentials_svc: None,
        }
    }

    #[tokio::test]
    async fn provider_token_kind_secret_satisfies_preflight() {
        // A PAT this backend validated against GitHub is as trustworthy as a
        // provider_tokens row, so preflight must accept it.
        let tokens: Arc<dyn ProviderTokenRepository> =
            Arc::new(StubProviderTokens::with_user_row(None));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_validated",
        )]));

        let token = resolve_task_github_token(github_inputs(&tokens, Some(&secrets))).await;
        assert_eq!(token, Some("ghp_validated".to_string()));
    }

    #[tokio::test]
    async fn env_kind_secret_does_not_satisfy_preflight() {
        // A plain `env` GITHUB_TOKEN is the operator escape hatch: deliberately
        // unvalidated, so preflight must not treat it as a verified credential.
        // `resolve_task_credentials` still injects it into the sandbox.
        let tokens: Arc<dyn ProviderTokenRepository> =
            Arc::new(StubProviderTokens::with_user_row(None));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "env",
            GITHUB_TOKEN,
            "ghp_unvalidated",
        )]));

        let token = resolve_task_github_token(github_inputs(&tokens, Some(&secrets))).await;
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn provider_tokens_row_still_wins_over_the_secret() {
        let tokens: Arc<dyn ProviderTokenRepository> =
            Arc::new(StubProviderTokens::with_user_row(Some(
                crate::auth::test_support::github_row("oauth", "gho_from_row"),
            )));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_from_secret",
        )]));

        let token = resolve_task_github_token(github_inputs(&tokens, Some(&secrets))).await;
        assert_eq!(token, Some("gho_from_row".to_string()));
    }

    #[tokio::test]
    async fn secret_ref_row_resolves_via_the_secret_not_the_sentinel() {
        // No App registry wired: the fallback dispatch still has to honour
        // `secret_ref`, or the sandbox would receive the sentinel string.
        let tokens: Arc<dyn ProviderTokenRepository> = Arc::new(StubProviderTokens::with_user_row(
            Some(crate::auth::test_support::github_row(
                AUTH_METHOD_SECRET_REF,
                crate::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER,
            )),
        ));
        let secrets = Arc::new(service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_from_secret",
        )]));

        let token = resolve_task_github_token(github_inputs(&tokens, Some(&secrets))).await;
        assert_eq!(token, Some("ghp_from_secret".to_string()));
    }

    #[tokio::test]
    async fn secret_ref_row_without_a_secret_service_resolves_to_none() {
        // Pins the trap that made preflight report a connected PAT as "GitHub
        // account not connected": with no secret service the sentinel cannot be
        // dereferenced, and the error is swallowed into an absent credential.
        // Callers must wire `secrets` — see `preflight_validate_github_token`.
        let tokens: Arc<dyn ProviderTokenRepository> = Arc::new(StubProviderTokens::with_user_row(
            Some(crate::auth::test_support::github_row(
                AUTH_METHOD_SECRET_REF,
                crate::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER,
            )),
        ));

        let token = resolve_task_github_token(github_inputs(&tokens, None)).await;
        assert_eq!(token, None);
    }

    // --- GITHUB_TOKEN scope precedence: user > team > global ---

    fn github_inputs_in_team<'a>(
        provider_tokens: &'a Arc<dyn ProviderTokenRepository>,
        team_id: &'a str,
    ) -> TaskCredentialInputs<'a> {
        TaskCredentialInputs {
            team_id: Some(team_id),
            ..github_inputs(provider_tokens, None)
        }
    }

    #[tokio::test]
    async fn user_github_token_wins_over_the_team_one() {
        // Regression pin: team credentials used to be filled before user ones,
        // so a team row silently shadowed the user's own token.
        let tokens: Arc<dyn ProviderTokenRepository> = Arc::new(
            StubProviderTokens::with_user_row(Some(crate::auth::test_support::github_row(
                "oauth", "gho_mine",
            )))
            .with_team_row(
                "t1",
                crate::auth::test_support::github_row("manual", "ghp_team"),
            ),
        );

        let token = resolve_task_github_token(github_inputs_in_team(&tokens, "t1")).await;
        assert_eq!(token, Some("gho_mine".to_string()));
    }

    #[tokio::test]
    async fn global_github_token_backstops_a_user_with_none() {
        let tokens: Arc<dyn ProviderTokenRepository> = Arc::new(
            StubProviderTokens::default()
                .with_global_row(crate::auth::test_support::github_row("manual", "ghp_org")),
        );

        let token = resolve_task_github_token(github_inputs(&tokens, None)).await;
        assert_eq!(token, Some("ghp_org".to_string()));
    }

    #[tokio::test]
    async fn team_github_secret_ref_never_delivers_the_sentinel() {
        // Team rows used to be read via `find_team_token`, which returns
        // `access_token` verbatim — for a reference row that is the sentinel.
        let tokens: Arc<dyn ProviderTokenRepository> =
            Arc::new(StubProviderTokens::default().with_team_row(
                "t1",
                crate::auth::test_support::github_row(
                    AUTH_METHOD_SECRET_REF,
                    crate::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER,
                ),
            ));
        let secrets = Arc::new(service_with(vec![secret(
            "team",
            Some("t1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_team_secret",
        )]));

        let token = resolve_task_github_token(TaskCredentialInputs {
            secrets: Some(&secrets),
            ..github_inputs_in_team(&tokens, "t1")
        })
        .await;
        assert_eq!(token, Some("ghp_team_secret".to_string()));
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
