//! Task credential resolution for sandbox runtime consumers.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_core::secret_keys::{
    ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN, ANTHROPIC_BASE_URL, CLAUDE_CODE_OAUTH_TOKEN,
    CODEX_AUTH_JSON, GITHUB_TOKEN, GITLAB_TOKEN, OPENAI_API_KEY, OPENAI_BASE_URL,
    is_forgejo_credential_key,
};
use tracing::warn;

use moatless_core::secret_keys::is_reserved_key;
use moatless_types::access_control::Actor;
use moatless_types::credential_key::{GH_TOKEN, GIT_CO_AUTHOR, GIT_TOKEN};
use moatless_workspace_types::repository::GitHostRepo;
use moatless_workspace_types::repository::RepositoryProvider as GitHostProvider;

use crate::lifecycle::ingress::{SandboxIdentity, SandboxIngress};
use moatless_ports::identity::{
    CommitIdentity, GitHostTokenError, GitHostTokenIssuer, HarnessCredentials, SecretResolver,
};
use moatless_ports::sandbox::CodexCredentials;
use moatless_types::store::StoreError;
use moatless_workspace_ports::{WorkspaceCredentialReader, WorkspaceResolver};
use moatless_workspace_types::repository::Repository;

pub const GIT_USER_NAME: &str = "GIT_USER_NAME";
pub const GIT_USER_EMAIL: &str = "GIT_USER_EMAIL";

pub struct TaskCredentialInputs<'a> {
    pub task_id: &'a str,
    /// The identity the task acts as, and the Workspace it acts on behalf of.
    /// Taken whole (not just a user id) so the git-host token resolved here
    /// matches what every other caller resolves for this task.
    pub actor: &'a Actor,
    pub secrets: Option<&'a Arc<dyn SecretResolver>>,
    /// The one way to a git-host token. Supplied once at the composition root,
    /// so this struct threads no credential ports of its own.
    pub git_host_tokens: Option<&'a Arc<dyn GitHostTokenIssuer>>,
    /// The actor's git author identity — the source of `GIT_USER_NAME` /
    /// `GIT_USER_EMAIL`. Bots get both; human users get their account email
    /// when set (name still resolves via the GitHub API).
    pub users: Option<&'a Arc<dyn CommitIdentity>>,
    /// Provider credentials pinned to the task's Workspace, decrypted and
    /// keyed by env var — resolve with [`resolve_workspace_credentials`].
    ///
    /// Highest precedence: every other source below fills only the slots these
    /// leave empty.
    pub workspace_credentials: HashMap<String, String>,
    /// The Workspace's own `env_vars` — plain configuration rather than
    /// credentials — resolved with [`resolve_workspace_env_vars`].
    ///
    /// Workspace scope outranks user- and global-scoped secrets; a pinned
    /// Workspace credential still wins a key they both name.
    pub workspace_env_vars: HashMap<String, String>,
}

/// Resolve the credentials a task's sandbox needs.
///
/// No GitHub, Forgejo or model credential is ever returned, whatever its
/// source: the pod reaches each through a backend proxy or the model gateway,
/// which holds the credential instead of handing it to the pod.
pub async fn resolve_task_credentials(input: TaskCredentialInputs<'_>) -> HashMap<String, String> {
    let mut credentials: HashMap<String, String> = input.workspace_credentials;
    for (key, value) in input.workspace_env_vars {
        // Guarded here too, for the same reason `fill_missing_from_scoped`
        // guards its own merge: this is where a platform-owned key would
        // take effect, and `fill_git_host_tokens` below fills only an
        // empty slot — a git-host key arriving here would suppress the
        // validated resolver, not just outrank it.
        if is_platform_owned_key(&key) {
            continue;
        }
        credentials.entry(key).or_insert(value);
    }

    let user_id = input.actor.user_id();
    fill_git_host_tokens(&mut credentials, input.git_host_tokens, input.actor).await;

    fill_git_identity(&mut credentials, input.users, user_id).await;
    if let Some(person) = input.actor.person() {
        fill_co_author(&mut credentials, input.users, person).await;
    }

    let scoped = resolve_scoped_secrets(input.secrets, user_id, input.task_id).await;

    fill_missing_from_scoped(&mut credentials, scoped);
    // Screened last rather than per source: a Workspace pin, a per-host PAT
    // secret, an OAuth stash and a model key all reach this map by different
    // routes, and none of them may leave the backend.
    credentials.retain(|key, _| !is_proxied_credential_key(key));
    credentials
}

/// The Anthropic keys the model gateway calls upstream with for `actor`.
///
/// Precedence: a Workspace pin, then Workspace `env_vars`, then the connected
/// Claude account, then scoped secrets.
pub async fn resolve_task_anthropic_credentials(
    actor: &Actor,
    secrets: Option<&Arc<dyn SecretResolver>>,
    provider_tokens: Option<&Arc<dyn HarnessCredentials>>,
    workspace_credentials: HashMap<String, String>,
    workspace_env_vars: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut credentials = workspace_credentials;
    for (key, value) in workspace_env_vars {
        if !is_platform_owned_key(&key) {
            credentials.entry(key).or_insert(value);
        }
    }
    let user_id = actor.user_id();
    if let Some(provider_tokens) = provider_tokens {
        fill_claude_token(&mut credentials, provider_tokens, user_id).await;
    }
    let scoped = resolve_scoped_secrets(secrets, user_id, "").await;
    fill_missing_from_scoped(&mut credentials, scoped);
    credentials.retain(|key, _| {
        matches!(
            key.as_str(),
            ANTHROPIC_API_KEY | ANTHROPIC_AUTH_TOKEN | ANTHROPIC_BASE_URL | CLAUDE_CODE_OAUTH_TOKEN
        )
    });
    credentials
}

/// The OpenAI keys the model gateway's Codex upstream is called with for
/// `actor`, with the precedence of [`resolve_task_anthropic_credentials`].
///
/// The Codex login is refreshed through `codex` first: the gateway is its only
/// presenter.
pub async fn resolve_task_openai_credentials(
    actor: &Actor,
    secrets: Option<&Arc<dyn SecretResolver>>,
    provider_tokens: Option<&Arc<dyn HarnessCredentials>>,
    codex: Option<&Arc<dyn CodexCredentials>>,
    workspace_credentials: HashMap<String, String>,
    workspace_env_vars: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut credentials = workspace_credentials;
    for (key, value) in workspace_env_vars {
        if !is_platform_owned_key(&key) {
            credentials.entry(key).or_insert(value);
        }
    }
    let user_id = actor.user_id();
    match codex {
        Some(codex) if !credentials.contains_key(CODEX_AUTH_JSON) => {
            match codex.ensure_fresh_codex_for(user_id).await {
                Ok(Some(blob)) => {
                    credentials.insert(CODEX_AUTH_JSON.to_string(), blob);
                }
                Ok(None) => {}
                Err(e) => warn!(error = %e, user_id, "ensure_fresh_codex_for failed"),
            }
        }
        _ => {}
    }
    if let Some(provider_tokens) = provider_tokens {
        fill_openai_key(&mut credentials, provider_tokens, user_id).await;
    }
    let scoped = resolve_scoped_secrets(secrets, user_id, "").await;
    fill_missing_from_scoped(&mut credentials, scoped);
    credentials.retain(|key, _| {
        matches!(
            key.as_str(),
            OPENAI_API_KEY | OPENAI_BASE_URL | CODEX_AUTH_JSON
        )
    });
    credentials
}

/// Whether `key` names a credential only the backend's proxies and model
/// gateway present. A Workspace pins `GITHUB_TOKEN` like any other provider
/// key.
fn is_proxied_credential_key(key: &str) -> bool {
    matches!(key, GITHUB_TOKEN | GH_TOKEN)
        || is_forgejo_credential_key(key)
        || is_model_credential_key(key)
}

/// Whether `key` is one the model gateway calls upstream with, which no
/// channel reaching a pod may carry.
///
/// A base URL counts: the gateway reads it as the upstream, and in a pod it
/// would point the agent past the gateway.
pub fn is_model_credential_key(key: &str) -> bool {
    matches!(
        key,
        ANTHROPIC_API_KEY
            | ANTHROPIC_AUTH_TOKEN
            | ANTHROPIC_BASE_URL
            | CLAUDE_CODE_OAUTH_TOKEN
            | OPENAI_API_KEY
            | OPENAI_BASE_URL
            | CODEX_AUTH_JSON
    )
}

/// Resolve the GitHub token a task's run-as identity presents through the
/// backend's GitHub proxy.
///
/// The preflight resolves here too, so a task it admits is one the proxy
/// serves.
pub async fn resolve_task_github_token(
    git_host_tokens: Option<&Arc<dyn GitHostTokenIssuer>>,
    actor: &Actor,
    repository: Option<&Repository>,
) -> Option<Result<String, GitHostTokenError>> {
    let repo = task_git_host_repo(repository)?;
    if repo.provider() != GitHostProvider::Github {
        return None;
    }
    Some(
        git_host_tokens?
            .token(actor, &repo.credential_account())
            .await,
    )
}

/// Resolve the token a task's identity presents for `repository`, whatever
/// provider hosts it; `None` when the repository names no parseable remote.
///
/// Unlike [`resolve_task_github_token`], answers for every provider: callers
/// prove a credential resolves and never hand one to a pod.
///
/// The preflight and the git proxy both resolve here, so they cannot disagree.
pub async fn resolve_task_git_token(
    git_host_tokens: Option<&Arc<dyn GitHostTokenIssuer>>,
    actor: &Actor,
    repository: Option<&Repository>,
) -> Option<Result<String, GitHostTokenError>> {
    let repo = task_git_host_repo(repository)?;
    Some(
        git_host_tokens?
            .token(actor, &repo.credential_account())
            .await,
    )
}

/// Merge scoped-secret values into `credentials` only for keys that aren't
/// already covered by a `provider_token`. Extracted so the priority logic is
/// directly unit-testable without standing up the full `SecretService`.
fn fill_missing_from_scoped(
    credentials: &mut HashMap<String, String>,
    scoped: HashMap<String, String>,
) {
    for (key, value) in scoped {
        // The whole [`is_git_auth_key`] set, not just `GITHUB_TOKEN`: a scoped
        // `env` secret named `GIT_TOKEN` or `GH_TOKEN` reaches git exactly the
        // same way, and this guard exists so preflight and runtime delivery
        // cannot disagree about which token a sandbox holds. `GIT_CO_AUTHOR`
        // is skipped so only `fill_co_author`'s screened value reaches a trailer.
        if is_git_auth_key(&key) || key == GIT_CO_AUTHOR {
            continue;
        }
        credentials.entry(key).or_insert(value);
    }
}

/// The Workspace a task's credentials are resolved against, and the run-as
/// identity its Sandbox pins.
#[derive(Default)]
pub struct TaskCredentialTargets {
    pub workspace_id: Option<String>,
    /// The identities [`resolve_task_actor`] takes off the Sandbox row.
    pub identity: SandboxIdentity,
}

/// Resolve the Workspace and run-as identity a task's sandbox draws
/// credentials against, both off the task's Sandbox row.
pub async fn resolve_task_credential_targets(
    sandboxes: Option<&Arc<SandboxIngress>>,
    task_id: &str,
) -> TaskCredentialTargets {
    let Some(sandboxes) = sandboxes else {
        return TaskCredentialTargets::default();
    };
    let Ok(Some(intent)) = sandboxes.intent(task_id).await else {
        return TaskCredentialTargets::default();
    };
    let identity = match sandboxes.identity(&intent.sandbox_id).await {
        Ok(identity) => identity.unwrap_or_default(),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to read the sandbox run-as identity");
            SandboxIdentity::default()
        }
    };
    TaskCredentialTargets {
        workspace_id: Some(intent.workspace_id),
        identity,
    }
}

/// Resolve the Workspace's own `env_vars` — configuration, not credentials (see
/// [`resolve_workspace_credentials`] for those).
///
/// Platform-owned keys are dropped; an unreadable Workspace yields no vars,
/// narrowing access rather than widening it.
pub async fn resolve_workspace_env_vars(
    workspaces: Option<&Arc<dyn WorkspaceResolver>>,
    workspace_id: Option<&str>,
) -> HashMap<String, String> {
    let (Some(workspaces), Some(workspace_id)) = (workspaces, workspace_id) else {
        return HashMap::new();
    };
    let resolved = match workspaces.resolve(workspace_id).await {
        Ok(workspace) => workspace.and_then(|workspace| workspace.env_vars),
        Err(e) => {
            warn!(error = %e, workspace_id, "Failed to read the workspace env vars");
            None
        }
    };
    let mut env_vars = resolved.unwrap_or_default();
    env_vars.retain(|key, _| !is_platform_owned_key(key));
    env_vars
}

/// Keys a Workspace's `env_vars` must never define on any channel reaching a
/// pod: reserved runtime keys, every git-auth key ([`is_git_auth_key`]) and
/// [`GIT_CO_AUTHOR`], which only the backend may set.
pub fn is_platform_owned_key(key: &str) -> bool {
    is_reserved_key(key) || is_git_auth_key(key) || key == GIT_CO_AUTHOR
}

/// Every git-authentication key minted for a task's run-as identity.
///
/// A Workspace value under one would win: [`fill_git_host_tokens`] fills only
/// unset keys, `credentials::fetch` keeps a Workspace `GIT_TOKEN` when no
/// provider token resolves, and a `GH_TOKEN` in a pod's env would hand the
/// agent a GitHub token no resolver validated.
pub fn is_git_auth_key(key: &str) -> bool {
    matches!(key, GITHUB_TOKEN | GITLAB_TOKEN | GH_TOKEN | GIT_TOKEN)
}

/// The identity a Task's sandbox acts as on a git host: `task_run_as` when set,
/// else the Workspace's `run_as_user_id`, else `person`.
///
/// `person` is [`SandboxIdentity::acting_user_id`], carried as any other
/// identity's [`Actor::person`]. An unreadable Workspace degrades to `person`;
/// a driver with no git-host connection resolves no token, never the owner's.
pub async fn resolve_task_actor(
    workspaces: Option<&Arc<dyn WorkspaceResolver>>,
    workspace_id: Option<&str>,
    task_run_as: Option<&str>,
    person: &str,
) -> Actor {
    run_as_identity(workspaces, workspace_id, task_run_as, person)
        .await
        .on_behalf_of(Some(person))
}

async fn run_as_identity(
    workspaces: Option<&Arc<dyn WorkspaceResolver>>,
    workspace_id: Option<&str>,
    task_run_as: Option<&str>,
    person: &str,
) -> Actor {
    let owner = Actor::of(person).on_workspace(workspace_id);
    // A Task naming its own run-as answers the question outright; the Workspace
    // is consulted only when it does not. `Actor::run_as` folds the owner
    // fallback in, so it is never `None` here and cannot be the branch's guard.
    if let Some(task_run_as) = task_run_as.filter(|id| !id.is_empty()) {
        return Actor::of(task_run_as).on_workspace(workspace_id);
    }
    let (Some(workspaces), Some(workspace_id)) = (workspaces, workspace_id) else {
        return owner;
    };
    let run_as = match workspaces.resolve(workspace_id).await {
        Ok(workspace) => workspace.and_then(|workspace| workspace.run_as_user_id),
        Err(e) => {
            warn!(error = %e, workspace_id, "Failed to read the workspace run-as identity");
            None
        }
    };
    Actor::run_as(run_as.as_deref(), Some(owner.clone()))
        .unwrap_or(owner)
        .on_workspace(Some(workspace_id))
}

/// Resolve the credentials pinned to a workspace, keyed by env var.
///
/// No workspace, or no service wired, yields an empty map; but credentials that
/// exist and cannot be read are an error, not a fallback, since that would run
/// the task under a different identity than configured.
pub async fn resolve_workspace_credentials(
    workspace_credentials: Option<&Arc<dyn WorkspaceCredentialReader>>,
    workspace_id: Option<&str>,
) -> Result<HashMap<String, String>, StoreError> {
    let (Some(service), Some(workspace_id)) = (workspace_credentials, workspace_id) else {
        return Ok(HashMap::new());
    };
    service.resolve_env(workspace_id).await
}

async fn resolve_scoped_secrets(
    secrets: Option<&Arc<dyn SecretResolver>>,
    user_id: &str,
    task_id: &str,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };

    match secrets.plaintext(user_id).await {
        Ok(credentials) => credentials,
        Err(e) => {
            warn!(task_id, error = %e, "Failed to resolve scoped secrets");
            HashMap::new()
        }
    }
}

/// The repository a task's git-host token is minted against.
fn task_git_host_repo(repository: Option<&Repository>) -> Option<&GitHostRepo> {
    let repository = repository?;
    repository.remote.as_ref()?.host_repo.as_ref()
}

/// Resolve the git-host tokens this task's sandbox needs, from the actor's own
/// rows or its Workspace's pin.
///
/// Only `GITLAB_TOKEN`, filled whenever held since a workspace may mount
/// several hosts' repos. GitHub is answered by the backend's proxies.
async fn fill_git_host_tokens(
    credentials: &mut HashMap<String, String>,
    git_host_tokens: Option<&Arc<dyn GitHostTokenIssuer>>,
    actor: &Actor,
) {
    let Some(git_host_tokens) = git_host_tokens else {
        return;
    };

    if !credentials.contains_key(GITLAB_TOKEN) {
        insert_token(
            credentials,
            GITLAB_TOKEN,
            actor,
            git_host_tokens
                .token(actor, &moatless_types::git_host::GitHostAccount::Gitlab)
                .await,
        );
    }
}

/// Store a resolved token, or account for why there is none.
///
/// Not-connected is silent (preflight turns that into a refusal); any other
/// failure is a broken credential an operator needs to see.
fn insert_token(
    credentials: &mut HashMap<String, String>,
    key: &str,
    actor: &Actor,
    resolved: Result<String, GitHostTokenError>,
) {
    match resolved {
        Ok(token) => {
            credentials.insert(key.to_string(), token);
        }
        Err(GitHostTokenError::NoCredential { .. }) => {}
        Err(e) => {
            warn!(error = %e, actor = %actor, key, "Failed to resolve a git-host token for the sandbox");
        }
    }
}

/// Fill `OPENAI_API_KEY` from the user's stored OpenAI key, unless one already
/// resolved.
async fn fill_openai_key(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn HarnessCredentials>,
    user_id: &str,
) {
    if credentials.contains_key(OPENAI_API_KEY) {
        return;
    }
    match provider_tokens.provider_token(user_id, "openai").await {
        Ok(Some(token)) => {
            credentials.insert(OPENAI_API_KEY.to_string(), token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, user_id, "Failed to query openai provider_token");
        }
    }
}

/// Fill `CLAUDE_CODE_OAUTH_TOKEN` from the user's connected Claude account,
/// unless a Claude credential already resolved.
async fn fill_claude_token(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn HarnessCredentials>,
    user_id: &str,
) {
    if credentials.contains_key(CLAUDE_CODE_OAUTH_TOKEN)
        || credentials.contains_key(ANTHROPIC_API_KEY)
    {
        return;
    }
    match provider_tokens.provider_token(user_id, "claude").await {
        Ok(Some(token)) => {
            credentials.insert(CLAUDE_CODE_OAUTH_TOKEN.to_string(), token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, user_id, "Failed to query claude provider_token");
        }
    }
}

/// Source the git author identity for the task's sandbox from the owning
/// `users` row, as `GIT_USER_NAME`/`GIT_USER_EMAIL` env vars.
///
/// Bot users get both fields from the row; human users get only the email
/// override (when set), leaving the name to GitHub-API resolution.
async fn fill_git_identity(
    credentials: &mut HashMap<String, String>,
    users: Option<&Arc<dyn CommitIdentity>>,
    user_id: &str,
) {
    let Some(users) = users else { return };
    // An actor that is not a user (the `system` sentinel, a `bot:` value) has
    // no git identity, and neither does one whose row is gone.
    let user = match users.git_identity(user_id).await {
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
    // Human user: their Moatless account email when set, else the sandbox's
    // GitHub-API resolution at init. The name falls back to the login rather
    // than to that resolution: it is fixed at init, so after a driver switch
    // it would name the previous driver.
    let name = user
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(user.login);
    credentials.insert(GIT_USER_NAME.to_string(), name);
    if let Some(email) = user.email {
        credentials.insert(GIT_USER_EMAIL.to_string(), email);
    }
}

/// Set [`GIT_CO_AUTHOR`] to `person`'s account name, falling back to their
/// login, and email.
///
/// Left unset for a bot, for a person with no email, and for a name or email
/// that could break out of the one trailer line the sandbox writes it into.
async fn fill_co_author(
    credentials: &mut HashMap<String, String>,
    users: Option<&Arc<dyn CommitIdentity>>,
    person: &str,
) {
    let Some(users) = users else { return };
    let user = match users.git_identity(person).await {
        Ok(Some(u)) if !u.is_bot => u,
        Ok(_) => return,
        Err(e) => {
            warn!(error = %e, person, "Failed to look up the co-author's git identity");
            return;
        }
    };
    let Some(email) = user.email else { return };
    let name = user
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(user.login);
    let unsafe_char = |c: char| c.is_control() || c == '<' || c == '>';
    if name.contains(unsafe_char) || email.contains(unsafe_char) || email.trim().is_empty() {
        return;
    }
    credentials.insert(
        GIT_CO_AUTHOR.to_string(),
        format!("{} <{}>", name.trim(), email.trim()),
    );
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

    // --- preflight trust: what the sandbox receives for GITHUB_TOKEN ---
    //
    // Which stored row or secret an identity's token comes from is the
    // issuer's question, asserted where the issuer lives. What is asserted
    // here is the flattening around it: a failure leaves the slot empty, and a
    // pinned Workspace credential outranks every other source.

    use moatless_ports::identity::{
        MockCommitIdentity, MockGitHostTokenIssuer, MockHarnessCredentials, MockSecretResolver,
    };

    fn github_repository(scope: moatless_types::access_control::Scope) -> Repository {
        Repository {
            id: "repo-1".to_string(),
            name: "repo".to_string(),
            kind: moatless_workspace_types::repository::RepositoryKind::Remote,
            remote: Some(moatless_workspace_types::repository::RepositoryRemote {
                url: "https://github.com/acme/repo.git".to_string(),
                host_repo: Some(moatless_workspace_types::repository::GitHostRepo::new(
                    moatless_workspace_types::repository::RepositoryProvider::Github,
                    "github.com",
                    "acme",
                    "repo",
                )),
            }),
            default_branch: None,
            description: None,
            provider: Some(moatless_workspace_types::repository::RepositoryProvider::Github),
            scope,
            owner_user_id: Some("u1".to_string()),
            created_by: Some("u1".to_string()),
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    fn no_credential(actor: &Actor, provider: GitHostProvider) -> GitHostTokenError {
        GitHostTokenError::NoCredential {
            actor: actor.clone(),
            provider,
            host: None,
        }
    }

    /// An issuer that answers every host with `token`, or holds nothing.
    fn issuer(token: Option<&'static str>) -> Arc<dyn GitHostTokenIssuer> {
        let mut issuer = MockGitHostTokenIssuer::new();
        issuer.expect_token().returning(move |actor, repo| {
            token
                .map(str::to_string)
                .ok_or_else(|| no_credential(actor, repo.provider()))
        });
        Arc::new(issuer)
    }

    fn secrets_holding(entries: &[(&str, &str)]) -> Arc<dyn SecretResolver> {
        let resolved: HashMap<String, String> = entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let mut secrets = MockSecretResolver::new();
        secrets
            .expect_plaintext()
            .returning(move |_| Ok(resolved.clone()));
        Arc::new(secrets)
    }

    /// Inputs for the whole-chain test, with credentials pinned to the task's
    /// workspace.
    fn workspace_github_inputs<'a>(
        actor: &'a Actor,
        git_host_tokens: &'a Arc<dyn GitHostTokenIssuer>,
        secrets: Option<&'a Arc<dyn SecretResolver>>,
        workspace_credentials: HashMap<String, String>,
    ) -> TaskCredentialInputs<'a> {
        TaskCredentialInputs {
            task_id: "t1",
            actor,
            secrets,
            git_host_tokens: Some(git_host_tokens),
            users: None,
            workspace_credentials,
            workspace_env_vars: HashMap::new(),
        }
    }

    // --- the workspace names the identity every task on it acts as ---

    fn workspace_running_as(run_as_user_id: Option<&str>) -> Arc<dyn WorkspaceResolver> {
        workspace_resolving_to(run_as_user_id, None)
    }

    /// A resolver whose single Workspace carries the given run-as identity and
    /// `env_vars`.
    fn workspace_resolving_to(
        run_as_user_id: Option<&str>,
        env_vars: Option<HashMap<String, String>>,
    ) -> Arc<dyn WorkspaceResolver> {
        let run_as = run_as_user_id.map(str::to_string);
        let mut resolver = moatless_workspace_ports::MockWorkspaceResolver::new();
        resolver.expect_resolve().returning(move |id| {
            Ok(Some(moatless_workspace_ports::ResolvedWorkspace {
                id: id.to_string(),
                name: "ws".to_string(),
                docker_image: None,
                docker_access: None,
                sandbox_service_account: None,
                system_prompt: None,
                setup_commands: None,
                env_vars: env_vars.clone(),
                resources: None,
                language_servers: None,
                network_profile: None,
                node_selector: None,
                run_as_user_id: run_as.clone(),
                deleted: false,
            }))
        });
        Arc::new(resolver)
    }

    // --- workspace > user > global for configuration ---

    #[tokio::test]
    async fn workspace_env_vars_outrank_a_scoped_secret_of_the_same_key() {
        let workspaces = workspace_resolving_to(
            None,
            Some(HashMap::from([(
                "GH_PACKAGES_TOKEN".to_string(),
                "from-workspace".to_string(),
            )])),
        );
        let workspace_env_vars = resolve_workspace_env_vars(Some(&workspaces), Some("ws_1")).await;

        let mut credentials: HashMap<String, String> = HashMap::new();
        for (key, value) in workspace_env_vars {
            credentials.entry(key).or_insert(value);
        }
        // A user- or global-scoped secret arrives through this path, and fills
        // only what the workspace left open.
        fill_missing_from_scoped(
            &mut credentials,
            HashMap::from([
                ("GH_PACKAGES_TOKEN".to_string(), "from-scope".to_string()),
                ("GRAFANA_TOKEN".to_string(), "from-scope".to_string()),
            ]),
        );

        assert_eq!(credentials["GH_PACKAGES_TOKEN"], "from-workspace");
        assert_eq!(credentials["GRAFANA_TOKEN"], "from-scope");
    }

    #[tokio::test]
    async fn a_workspace_cannot_redefine_a_reserved_runtime_key() {
        let workspaces = workspace_resolving_to(
            None,
            Some(HashMap::from([
                ("TASK_ID".to_string(), "hijacked".to_string()),
                ("USER_AUTH_TOKEN".to_string(), "hijacked".to_string()),
                ("CARGO_BUILD_JOBS".to_string(), "4".to_string()),
            ])),
        );

        let env_vars = resolve_workspace_env_vars(Some(&workspaces), Some("ws_1")).await;

        assert_eq!(
            env_vars,
            HashMap::from([("CARGO_BUILD_JOBS".to_string(), "4".to_string())])
        );
    }

    /// `fill_git_host_tokens` fills only a slot that is still empty, so a
    /// workspace-declared git-host token would suppress the validated resolver
    /// outright, not merely outrank it.
    #[tokio::test]
    async fn a_workspace_cannot_declare_a_git_host_token_as_plain_config() {
        let workspaces = workspace_resolving_to(
            None,
            Some(HashMap::from([
                (GITHUB_TOKEN.to_string(), "ghp_bypass".to_string()),
                (GITLAB_TOKEN.to_string(), "glpat_bypass".to_string()),
                // The two the pod derives rather than resolves: GH_TOKEN would
                // reach the agent's env, and init::git clones with GIT_TOKEN
                // whenever no provider token overwrote it.
                (GH_TOKEN.to_string(), "ghp_bypass".to_string()),
                (GIT_TOKEN.to_string(), "ghp_bypass".to_string()),
                ("CARGO_BUILD_JOBS".to_string(), "4".to_string()),
            ])),
        );

        let env_vars = resolve_workspace_env_vars(Some(&workspaces), Some("ws_1")).await;

        assert_eq!(
            env_vars,
            HashMap::from([("CARGO_BUILD_JOBS".to_string(), "4".to_string())])
        );
    }

    /// The merge refuses one too, so a caller assembling `workspace_env_vars`
    /// from somewhere other than the resolver cannot reopen the bypass.
    ///
    /// The user here holds a real row: without the guard the workspace value
    /// would occupy the slot before it is ever consulted.
    #[tokio::test]
    async fn a_workspace_env_git_host_token_never_suppresses_the_resolver() {
        let actor = Actor::of("u1");
        let git_host_tokens = issuer(Some("glpat_validated"));

        let mut inputs = workspace_github_inputs(&actor, &git_host_tokens, None, HashMap::new());
        inputs.workspace_env_vars = HashMap::from([
            (GITLAB_TOKEN.to_string(), "glpat_bypass".to_string()),
            (GH_TOKEN.to_string(), "ghp_bypass".to_string()),
            (GIT_TOKEN.to_string(), "ghp_bypass".to_string()),
        ]);

        let credentials = resolve_task_credentials(inputs).await;

        assert_eq!(credentials[GITLAB_TOKEN], "glpat_validated");
        assert!(!credentials.contains_key(GH_TOKEN));
        assert!(!credentials.contains_key(GIT_TOKEN));
    }

    /// The same names on the scoped-secret channel. `fill_missing_from_scoped`
    /// guarded only `GITHUB_TOKEN`, but a scoped `env` secret named `GIT_TOKEN`
    /// reaches git identically.
    #[tokio::test]
    async fn a_scoped_env_secret_cannot_supply_a_git_credential_under_an_alias() {
        let mut credentials = HashMap::new();
        fill_missing_from_scoped(
            &mut credentials,
            HashMap::from([
                (GH_TOKEN.to_string(), "ghp_unvalidated".to_string()),
                (GIT_TOKEN.to_string(), "ghp_unvalidated".to_string()),
                ("GRAFANA_TOKEN".to_string(), "glsa-fine".to_string()),
            ]),
        );

        assert_eq!(
            credentials,
            HashMap::from([("GRAFANA_TOKEN".to_string(), "glsa-fine".to_string())])
        );
    }

    #[tokio::test]
    async fn no_workspace_yields_no_env_vars() {
        assert!(
            resolve_workspace_env_vars(None, Some("ws_1"))
                .await
                .is_empty()
        );
        let workspaces = workspace_resolving_to(None, None);
        assert!(
            resolve_workspace_env_vars(Some(&workspaces), None)
                .await
                .is_empty()
        );
        assert!(
            resolve_workspace_env_vars(Some(&workspaces), Some("ws_1"))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_workspace_run_as_identity_beats_whoever_started_the_task() {
        // The point of the feature: the workspace names the account, not
        // whoever happened to start the task.
        let workspaces = workspace_running_as(Some("bot"));
        let actor = resolve_task_actor(Some(&workspaces), Some("ws_1"), None, "starter").await;
        assert_eq!(actor.user_id(), "bot");
    }

    #[tokio::test]
    async fn a_run_as_identity_carries_the_owner_whose_access_caps_it() {
        let workspaces = workspace_running_as(Some("bot"));
        let actor = resolve_task_actor(Some(&workspaces), Some("ws_1"), None, "starter").await;
        assert_eq!(actor.person(), Some("starter"));

        let actor =
            resolve_task_actor(Some(&workspaces), Some("ws_1"), Some("reviewer"), "starter").await;
        assert_eq!(actor.person(), Some("starter"));

        let workspaces = workspace_running_as(None);
        let actor = resolve_task_actor(Some(&workspaces), Some("ws_1"), None, "starter").await;
        assert_eq!(
            actor.person(),
            None,
            "the owner acting as itself is uncapped"
        );
    }

    #[tokio::test]
    async fn a_task_run_as_identity_beats_the_workspace_account() {
        // `moat tasks create --run-as` is one admin naming one bot for one
        // task; a Workspace account applies to everything that runs in it, so
        // the narrower statement wins.
        let workspaces = workspace_running_as(Some("bot"));
        let actor =
            resolve_task_actor(Some(&workspaces), Some("ws_1"), Some("reviewer"), "starter").await;
        assert_eq!(actor.user_id(), "reviewer");
        assert_eq!(actor.workspace_id(), Some("ws_1"));
    }

    #[tokio::test]
    async fn a_workspace_naming_nobody_leaves_the_task_owner_acting() {
        let workspaces = workspace_running_as(None);
        let actor = resolve_task_actor(Some(&workspaces), Some("ws_1"), None, "starter").await;
        assert_eq!(actor.user_id(), "starter");
    }

    #[tokio::test]
    async fn the_resolved_actor_carries_the_workspace_whatever_it_runs_as() {
        // What a pinned git-host credential is found from. Preflight, the
        // sandbox and every backend-side call resolve their token from this
        // actor, so losing the workspace here would leave them disagreeing
        // about which account the task presents.
        for run_as in [Some("bot"), None] {
            let workspaces = workspace_running_as(run_as);
            let actor = resolve_task_actor(Some(&workspaces), Some("ws_1"), None, "starter").await;
            assert_eq!(actor.workspace_id(), Some("ws_1"), "run_as {run_as:?}");
        }
    }

    #[tokio::test]
    async fn an_unreadable_workspace_still_names_itself_on_the_actor() {
        // The pin is looked up from the id alone, so a workspace row that
        // cannot be read narrows the run-as identity without also dropping the
        // credential the workspace pins.
        let actor = resolve_task_actor(None, Some("ws_1"), None, "starter").await;
        assert_eq!(actor.user_id(), "starter");
        assert_eq!(actor.workspace_id(), Some("ws_1"));
    }

    #[tokio::test]
    async fn a_task_with_no_workspace_acts_as_its_own_owner() {
        let workspaces = workspace_running_as(Some("bot"));
        assert_eq!(
            resolve_task_actor(Some(&workspaces), None, None, "starter")
                .await
                .user_id(),
            "starter"
        );
        assert_eq!(
            resolve_task_actor(None, Some("ws_1"), None, "starter")
                .await
                .user_id(),
            "starter"
        );
    }

    /// The pod reaches GitHub only through the backend's proxies, so no
    /// GitHub token reaches it on any channel, even one the issuer would mint.
    #[tokio::test]
    async fn no_github_credential_reaches_a_pod_whatever_its_source() {
        let mut reader = moatless_workspace_ports::MockWorkspaceCredentialReader::new();
        reader.expect_resolve_env().returning(|_| {
            Ok(HashMap::from([
                (GITHUB_TOKEN.to_string(), "ghp_workspace".to_string()),
                ("GRAFANA_TOKEN".to_string(), "glsa-workspace".to_string()),
            ]))
        });
        let reader: Arc<dyn WorkspaceCredentialReader> = Arc::new(reader);
        let pinned = resolve_workspace_credentials(Some(&reader), Some("ws_1"))
            .await
            .unwrap();
        let secrets = secrets_holding(&[(GITHUB_TOKEN, "ghp_secret"), (GH_TOKEN, "ghp_secret")]);
        let actor = Actor::of("u1").on_workspace(Some("ws_1"));

        let credentials = resolve_task_credentials(workspace_github_inputs(
            &actor,
            &issuer(Some("gho_from_user")),
            Some(&secrets),
            pinned,
        ))
        .await;

        assert!(!credentials.contains_key(GITHUB_TOKEN));
        assert!(!credentials.contains_key(GH_TOKEN));
        assert_eq!(credentials["GRAFANA_TOKEN"], "glsa-workspace");
        assert_eq!(
            credentials[GITLAB_TOKEN], "gho_from_user",
            "GitLab is still delivered"
        );
    }

    #[tokio::test]
    async fn a_pinned_credential_survives_the_whole_resolution_chain() {
        // End to end through `resolve_task_credentials`: the workspace value
        // must beat the user's provider_token *and* the scoped secret that
        // would otherwise fill the same slot.
        let pinned = HashMap::from([("GRAFANA_TOKEN".to_string(), "glsa-workspace".to_string())]);
        let secrets = secrets_holding(&[("GRAFANA_TOKEN", "glsa-from-secret")]);

        let actor = Actor::of("u1");
        let credentials = resolve_task_credentials(workspace_github_inputs(
            &actor,
            &issuer(None),
            Some(&secrets),
            pinned,
        ))
        .await;

        assert_eq!(credentials["GRAFANA_TOKEN"], "glsa-workspace");
    }

    #[tokio::test]
    async fn the_issuer_is_asked_on_behalf_of_the_workspace_the_actor_names() {
        // The issuer can only resolve a Workspace pin from the actor's
        // Workspace. `TaskCredentialInputs` takes the actor whole so the
        // Workspace survives the trip; an unseeded map means the issuer
        // supplied it.
        let mut mock = MockGitHostTokenIssuer::new();
        mock.expect_token()
            .returning(|actor, repo| match actor.workspace_id() {
                Some("ws_1") if repo.provider() == GitHostProvider::Gitlab => {
                    Ok("glpat_workspace".to_string())
                }
                _ => Err(no_credential(actor, repo.provider())),
            });
        let git_host_tokens: Arc<dyn GitHostTokenIssuer> = Arc::new(mock);
        let actor = Actor::of("u1").on_workspace(Some("ws_1"));

        let credentials = resolve_task_credentials(workspace_github_inputs(
            &actor,
            &git_host_tokens,
            None,
            HashMap::new(),
        ))
        .await;

        assert_eq!(credentials[GITLAB_TOKEN], "glpat_workspace");
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
    async fn an_identity_with_no_git_host_credential_gets_an_empty_slot() {
        // `fill_git_host_tokens` flattens every failure the issuer reports: the
        // sandbox starts without the variable rather than with somebody else's
        // token. Preflight is where the absence becomes a refusal.
        let repository = github_repository(moatless_types::access_control::Scope::User);
        let token =
            resolve_task_github_token(Some(&issuer(None)), &Actor::of("u1"), Some(&repository))
                .await
                .and_then(Result::ok);
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn a_scoped_secret_never_fills_the_git_host_slot() {
        // A `GITLAB_TOKEN` sitting in the secret store must not reach the
        // sandbox: the issuer is the one way to a git-host token, and it holds
        // nothing for this identity.
        let secrets = secrets_holding(&[(GITLAB_TOKEN, "glpat_unvalidated")]);
        let actor = Actor::of("u1");

        let credentials = resolve_task_credentials(workspace_github_inputs(
            &actor,
            &issuer(None),
            Some(&secrets),
            HashMap::new(),
        ))
        .await;
        assert!(!credentials.contains_key(GITHUB_TOKEN));
    }

    /// Forgejo is proxied, so the pod never holds the credential — on any of
    /// the three channels one arrives by. Asserted end to end rather than on
    /// the predicate, because each channel merges into the map separately.
    #[tokio::test]
    async fn no_forgejo_credential_reaches_a_pod_whatever_its_source() {
        use moatless_core::secret_keys::{FORGEJO_TOKEN, forgejo_oauth_stash_key};

        let stash = forgejo_oauth_stash_key("git.acme.dev");
        let secrets = secrets_holding(&[
            (stash.as_str(), "oauth-stash"),
            ("GRAFANA_TOKEN", "glsa-fine"),
        ]);
        let actor = Actor::of("u1");
        let issuer = issuer(None);

        let mut inputs = workspace_github_inputs(
            &actor,
            &issuer,
            Some(&secrets),
            HashMap::from([(FORGEJO_TOKEN.to_string(), "pinned".to_string())]),
        );
        inputs.workspace_env_vars = HashMap::from([(
            moatless_core::secret_keys::forgejo_token_key("git.acme.dev"),
            "pat".to_string(),
        )]);

        let credentials = resolve_task_credentials(inputs).await;

        assert_eq!(
            credentials,
            HashMap::from([("GRAFANA_TOKEN".to_string(), "glsa-fine".to_string())])
        );
    }

    /// Every model is reached through the backend's gateway, so no model key
    /// or base URL reaches a pod on any of the three channels one arrives by.
    #[tokio::test]
    async fn no_model_credential_reaches_a_pod_whatever_its_source() {
        let model_keys = [
            ANTHROPIC_API_KEY,
            ANTHROPIC_AUTH_TOKEN,
            ANTHROPIC_BASE_URL,
            CLAUDE_CODE_OAUTH_TOKEN,
            OPENAI_API_KEY,
            OPENAI_BASE_URL,
            CODEX_AUTH_JSON,
        ];
        let entries: Vec<(&str, &str)> = model_keys.iter().map(|key| (*key, "leaked")).collect();
        let secrets = secrets_holding(&entries);
        let actor = Actor::of("u1");
        let issuer = issuer(None);
        let as_map = |keys: &[&str]| -> HashMap<String, String> {
            keys.iter()
                .map(|key| ((*key).to_string(), "leaked".to_string()))
                .collect()
        };

        let mut inputs =
            workspace_github_inputs(&actor, &issuer, Some(&secrets), as_map(&model_keys));
        inputs.workspace_env_vars = as_map(&model_keys);

        let credentials = resolve_task_credentials(inputs).await;

        assert!(credentials.is_empty(), "delivered {credentials:?}");
    }

    /// The user's stored OpenAI key, which no pod receives, is what the
    /// gateway presents for them.
    #[tokio::test]
    async fn the_gateway_presents_the_users_stored_openai_key() {
        let mut stored = MockHarnessCredentials::new();
        stored
            .expect_provider_token()
            .withf(|user_id, provider| user_id == "u1" && provider == "openai")
            .returning(|_, _| Ok(Some("sk-stored".to_string())));
        let stored: Arc<dyn HarnessCredentials> = Arc::new(stored);

        let credentials = resolve_task_openai_credentials(
            &Actor::of("u1"),
            None,
            Some(&stored),
            None,
            HashMap::new(),
            HashMap::new(),
        )
        .await;

        assert_eq!(
            credentials.get(OPENAI_API_KEY).map(String::as_str),
            Some("sk-stored")
        );
    }

    /// A `CommitIdentity` reporting one git author identity for every lookup.
    fn users_returning(
        login: &'static str,
        name: Option<&'static str>,
        email: Option<&'static str>,
        is_bot: bool,
    ) -> Arc<dyn CommitIdentity> {
        let mut users = MockCommitIdentity::new();
        users.expect_git_identity().returning(move |_| {
            Ok(Some(moatless_ports::identity::GitIdentity {
                is_bot,
                login: login.to_string(),
                name: name.map(str::to_string),
                email: email.map(str::to_string),
            }))
        });
        Arc::new(users)
    }

    async fn co_author_of(users: &Arc<dyn CommitIdentity>) -> Option<String> {
        let mut credentials = HashMap::new();
        fill_co_author(&mut credentials, Some(users), "ada").await;
        credentials.remove(GIT_CO_AUTHOR)
    }

    #[tokio::test]
    async fn the_person_behind_a_run_as_identity_is_the_co_author() {
        let named = users_returning("ada", Some("Ada Lovelace"), Some("ada@example.com"), false);
        assert_eq!(
            co_author_of(&named).await.as_deref(),
            Some("Ada Lovelace <ada@example.com>")
        );
        let unnamed = users_returning("ada", None, Some("ada@example.com"), false);
        assert_eq!(
            co_author_of(&unnamed).await.as_deref(),
            Some("ada <ada@example.com>"),
            "a person with no account name is credited by login"
        );
    }

    #[tokio::test]
    async fn no_co_author_without_an_address_github_can_credit() {
        for users in [
            users_returning("ada", Some("Ada"), None, false),
            users_returning("bot", Some("Bot"), Some("bot@example.com"), true),
            users_returning(
                "ada",
                Some("Ada\nSigned-off-by: x"),
                Some("ada@example.com"),
                false,
            ),
            users_returning("ada", Some("Ada"), Some("ada@example.com> <x@y"), false),
        ] {
            assert_eq!(co_author_of(&users).await, None);
        }
    }

    #[test]
    fn only_the_backend_names_a_co_author() {
        assert!(is_platform_owned_key(GIT_CO_AUTHOR));
        let mut credentials = HashMap::new();
        fill_missing_from_scoped(
            &mut credentials,
            HashMap::from([(
                GIT_CO_AUTHOR.to_string(),
                "Eve <e@x>\nSigned-off-by: x".to_string(),
            )]),
        );
        assert!(credentials.is_empty());
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
    async fn human_user_without_a_name_commits_under_their_login() {
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
        assert_eq!(
            credentials.get(GIT_USER_NAME).map(String::as_str),
            Some("alice")
        );
    }

    #[tokio::test]
    async fn human_user_without_an_email_leaves_it_to_the_sandbox() {
        let users = users_returning("alice", None, None, /* is_bot */ false);

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert!(!credentials.contains_key(GIT_USER_EMAIL));
        assert_eq!(
            credentials.get(GIT_USER_NAME).map(String::as_str),
            Some("alice")
        );
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
