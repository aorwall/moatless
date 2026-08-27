//! Task credential resolution for sandbox runtime consumers.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_core::secret_keys::{ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, CODEX_AUTH_JSON, GITHUB_TOKEN, GITLAB_TOKEN, GITNESS_TOKEN, OPENAI_API_KEY};
use tracing::warn;

use moatless_types::repository::RepositoryProvider as GitHostProvider;
use moatless_types::access_control::Actor;
use moatless_identity::git_host_repo::GitHostRepo;
use moatless_identity::git_host_tokens::{GitHostTokenError, GitHostTokens, PlainGitHost};
use moatless_identity::secret::models::is_reserved_key;
use moatless_identity::secret::service::SecretService;

use crate::lifecycle::orchestrator::LifecycleDao;
use moatless_backend_core::identity::{CommitIdentity, HarnessCredentials};
use moatless_backend_core::sandbox::CodexCredentials;
use moatless_backend_core::store::StoreError;
use moatless_backend_core::workspace::{WorkspaceCredentialReader, WorkspaceResolver};
use moatless_types::repository::Repository;

pub const GIT_USER_NAME: &str = "GIT_USER_NAME";
pub const GIT_USER_EMAIL: &str = "GIT_USER_EMAIL";

pub struct TaskCredentialInputs<'a> {
    pub task_id: &'a str,
    /// The identity the task acts as, and the Workspace it acts on behalf of —
    /// [`resolve_task_actor`] resolves both, applying the Workspace's run-as
    /// override over the task's own owner. Taken whole rather than as a user
    /// id so the git-host token resolved here is the one every other caller
    /// resolves for this task.
    pub actor: &'a Actor,
    pub secrets: Option<&'a Arc<SecretService>>,
    pub provider_tokens: Option<&'a Arc<dyn HarnessCredentials>>,
    /// The one way to a git-host token. Supplied once at the composition root,
    /// so this struct threads no credential ports of its own.
    pub git_host_tokens: Option<&'a Arc<GitHostTokens>>,
    /// Persisted repository — supplies the remote URL whose owner and name the
    /// run-as identity's credential is minted against.
    pub repository: Option<&'a Repository>,
    /// The actor's git author identity — the source of `GIT_USER_NAME` /
    /// `GIT_USER_EMAIL`. Bots get both; human users get their account email
    /// when set (name still resolves via the GitHub API).
    pub users: Option<&'a Arc<dyn CommitIdentity>>,
    /// Codex credential orchestrator used to refresh codex tokens just-in-time
    /// before handing the blob to the sandbox. When `None`, the codex branch
    /// falls back to a raw provider_tokens read.
    pub credential_sync: Option<&'a Arc<dyn CodexCredentials>>,
    /// Provider credentials pinned to the task's Workspace, already decrypted
    /// and keyed by environment variable — resolve them with
    /// [`resolve_workspace_credentials`]. Highest precedence: every other
    /// source below fills only the slots these leave empty.
    pub workspace_credentials: HashMap<String, String>,
    /// The Workspace's own `env_vars` — plain configuration rather than
    /// credentials — resolved with [`resolve_workspace_env_vars`]. Workspace
    /// scope, so they outrank user- and global-scoped secrets; a pinned
    /// Workspace credential still wins a key they both name.
    pub workspace_env_vars: HashMap<String, String>,
}

/// Resolve the credentials a task's sandbox needs.
///
/// Priority order (most-specific wins):
/// 1. Credentials pinned to the task's Workspace. A Workspace credential names
///    the account the Workspace runs as, so it outranks whoever started the
///    task; without that it would not be an override. A git-host token among
///    them reaches the sandbox by this route and is also what
///    [`GitHostTokens`] resolves for backend-side calls, because the actor
///    carries the Workspace (see [`resolve_task_actor`]) — so the two agree on
///    which token the task presents.
/// 2. The Workspace's own `env_vars`. Same scope as 1 and above every
///    per-user source, so the resolved order across scopes is
///    workspace > user > global for configuration as well as credentials.
/// 3. Per-user `provider_tokens` for `user_id`, the identity the task acts as
///    (OAuth-managed, e.g. a freshly-issued Codex `auth.json` from the
///    device-code flow or a connected GitHub App user-token).
/// 4. Scoped secrets — the `SecretService` already resolves these with
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
    for (key, value) in input.workspace_env_vars {
        // Guarded here as well as at the resolver, for the same reason
        // `fill_missing_from_scoped` guards its own merge: this is where a
        // platform-owned key would take effect, and `fill_git_host_tokens`
        // below fills only a slot that is still empty — so a git-host key
        // arriving here would suppress the validated resolver, not just
        // outrank it.
        if is_platform_owned_key(&key) {
            continue;
        }
        credentials.entry(key).or_insert(value);
    }

    let user_id = input.actor.user_id();
    fill_git_host_tokens(
        &mut credentials,
        input.git_host_tokens,
        input.actor,
        input.repository,
    )
    .await;

    fill_provider_tokens(
        &mut credentials,
        input.provider_tokens,
        user_id,
        input.credential_sync,
        Some(input.task_id),
    )
    .await;

    fill_git_identity(&mut credentials, input.users, user_id).await;

    let scoped = resolve_scoped_secrets(input.secrets, user_id, input.task_id).await;

    fill_missing_from_scoped(&mut credentials, scoped);
    credentials
}

/// Resolve only the GitHub token a task's sandbox would receive.
///
/// Goes through the same run-as-identity resolution as runtime credential
/// delivery, so preflight and the sandbox never disagree. `user_id` is that
/// identity — see [`resolve_task_actor`], which is where a Workspace's
/// override is applied.
pub async fn resolve_task_github_token(
    git_host_tokens: Option<&Arc<GitHostTokens>>,
    actor: &Actor,
    repository: Option<&Repository>,
) -> Option<Result<String, GitHostTokenError>> {
    let repo = task_git_host_repo(repository)?;
    if repo.provider() != GitHostProvider::Github {
        return None;
    }
    Some(git_host_tokens?.token(actor, &repo).await)
}

/// Merge scoped-secret values into `credentials` only for keys that aren't
/// already covered by a provider_token. Extracted so the priority logic is
/// directly unit-testable without standing up the full `SecretService`.
fn fill_missing_from_scoped(
    credentials: &mut HashMap<String, String>,
    scoped: HashMap<String, String>,
) {
    for (key, value) in scoped {
        // The whole [`is_git_auth_key`] set, not just `GITHUB_TOKEN`: a scoped
        // `env` secret named `GIT_TOKEN` or `GH_TOKEN` reaches git exactly the
        // same way, and this guard exists so preflight and runtime delivery
        // cannot disagree about which token a sandbox holds.
        if is_git_auth_key(&key) {
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
/// The same workspace > user > global order the agent env resolves in, for the
/// same reason: a sidecar sources `server.env` after the pod environment, so a
/// workspace var delivered only through the pod spec would lose to a scoped
/// secret here too. Reserved runtime keys are filtered out. The agent env is a
/// superset that additionally includes provider tokens.
pub async fn resolve_server_env(
    user_id: &str,
    secrets: Option<&Arc<SecretService>>,
    workspace_env_vars: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut env = workspace_env_vars.clone();
    let Some(secrets) = secrets else {
        return env;
    };
    match secrets.resolve_accessible_plaintext(user_id).await {
        Ok(scoped) => {
            for (key, value) in scoped {
                env.entry(key).or_insert(value);
            }
        }
        Err(e) => {
            warn!(user_id, error = %e, "Failed to resolve server env secrets");
        }
    }
    env.retain(|key, _| !is_reserved_key(key));
    env
}

/// The two rows a task's credentials are resolved against: the repository whose
/// owner/name a GitHub credential is minted for, and the workspace whose pinned
/// credentials outrank everything else.
#[derive(Default)]
pub struct TaskCredentialTargets {
    pub repository: Option<Repository>,
    pub workspace_id: Option<String>,
}

/// Resolve the repository and workspace a task's sandbox draws credentials
/// against.
///
/// The workspace comes off the task's Sandbox row, and the repository is that
/// workspace's primary placement — the same repository the pod's primary
/// checkout was built from.
pub async fn resolve_task_credential_targets(
    sandboxes: Option<&Arc<dyn LifecycleDao>>,
    workspaces: Option<&Arc<dyn WorkspaceResolver>>,
    task_id: &str,
) -> TaskCredentialTargets {
    let (Some(sandboxes), Some(workspaces)) = (sandboxes, workspaces) else {
        return TaskCredentialTargets::default();
    };
    let Ok(Some(intent)) = sandboxes.load(task_id).await else {
        return TaskCredentialTargets::default();
    };
    let primary = match workspaces.placements(&intent.workspace_id).await {
        Ok(placements) => placements.into_iter().next(),
        Err(e) => {
            warn!(
                task_id,
                workspace_id = %intent.workspace_id,
                error = %e,
                "Failed to read the workspace's repository placements"
            );
            None
        }
    };
    TaskCredentialTargets {
        repository: primary.and_then(|placement| placement.repository),
        workspace_id: Some(intent.workspace_id),
    }
}

/// Resolve the Workspace's own `env_vars` — the configuration pinned to a
/// Workspace, as opposed to the credentials [`resolve_workspace_credentials`]
/// returns.
///
/// Platform-owned keys are dropped ([`is_platform_owned_key`]). A Workspace
/// that cannot be read yields no vars — the same degradation as
/// [`resolve_task_actor`], narrowing what a task gets rather than widening it.
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

/// Keys a Workspace's `env_vars` must never define, on any channel that reaches
/// a pod.
///
/// Two families, one rule — the platform names the value, so a Workspace naming
/// it too would be shadowing rather than configuring:
///
/// - Reserved runtime keys (`TASK_ID`, `USER_AUTH_TOKEN`, …): the pod's own
///   identity and wiring.
/// - Every key git authentication reaches the sandbox through
///   ([`is_git_auth_key`]).
///
/// A Workspace that legitimately runs as another git account sets
/// `run_as_user_id` and gets that identity's validated token — see
/// [`resolve_task_actor`].
pub fn is_platform_owned_key(key: &str) -> bool {
    is_reserved_key(key) || is_git_auth_key(key)
}

/// `GH_TOKEN` and `GIT_TOKEN`: not secret-store keys, so they are not in
/// `moatless_core::secret_keys`. `init::credentials` derives both inside the
/// pod from a resolved provider token — `GIT_TOKEN` so `init::git` has one env
/// var to clone with whatever the pod's provider is, `GH_TOKEN` as the name
/// `gh` reads. Named here because a Workspace must not be able to supply
/// either.
const GH_TOKEN: &str = "GH_TOKEN";
const GIT_TOKEN: &str = "GIT_TOKEN";

/// Every key that carries git authentication into a sandbox.
///
/// All five are minted for the task's run-as identity — the first three by
/// [`fill_git_host_tokens`], against the task's own repository; the other two
/// derived from those by `init::credentials` once inside the pod. None is
/// configuration, and a value a Workspace supplied under one of these names
/// would be used as the clone/push credential:
///
/// - `fill_git_host_tokens` fills only a key that is still unset, so a
///   Workspace-supplied provider token would not merely outrank the validated
///   resolver, it would suppress it. Preflight and runtime delivery would then
///   disagree about which token the sandbox holds.
/// - `init::credentials` sets `GIT_TOKEN` in the process env only *if* a
///   provider token resolved (`sandbox/src/init/credentials.rs:190`). With
///   none, a Workspace `GIT_TOKEN` that arrived in the pod spec is still
///   sitting there, and `init::git::provider_git_token` and `init::workspace`
///   read it straight out of the environment.
/// - `GH_TOKEN` is deliberately never `set_var`'d (`ENV_SET_BLOCKLIST`) so `gh`
///   authenticates from the refresher-owned `hosts.yml`. `gh` checks the
///   environment first, so a Workspace `GH_TOKEN` in the pod spec defeats that
///   outright — it is in the environment from the pod's first process.
pub fn is_git_auth_key(key: &str) -> bool {
    matches!(
        key,
        GITHUB_TOKEN | GITLAB_TOKEN | GITNESS_TOKEN | GH_TOKEN | GIT_TOKEN
    )
}

/// The identity a Task's sandbox acts as on a git host.
///
/// The Workspace's `run_as_user_id` when it names one, otherwise the identity
/// the Task is attributed to. A Workspace names the account it runs as, so it
/// outranks whoever started the Task.
///
/// The returned Actor carries the Workspace itself, so a credential the
/// Workspace pins for a git host is the one every call on this Task presents —
/// see [`Actor::workspace_id`]. That holds whether or not the Workspace row can
/// be read: the pin is looked up from the id alone.
///
/// A Workspace that cannot be read degrades to the Task's own owner: the
/// column is an override, and losing it narrows access rather than widening
/// it.
pub async fn resolve_task_actor(
    workspaces: Option<&Arc<dyn WorkspaceResolver>>,
    workspace_id: Option<&str>,
    owner_user_id: &str,
) -> Actor {
    let owner = Actor::of(owner_user_id).on_workspace(workspace_id);
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
    workspace_credentials: Option<&Arc<dyn WorkspaceCredentialReader>>,
    workspace_id: Option<&str>,
) -> Result<HashMap<String, String>, StoreError> {
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

async fn fill_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: Option<&Arc<dyn HarnessCredentials>>,
    user_id: &str,
    credential_sync: Option<&Arc<dyn CodexCredentials>>,
    current_task_id: Option<&str>,
) {
    let Some(provider_tokens) = provider_tokens else {
        return;
    };

    fill_model_provider_tokens(credentials, provider_tokens, user_id).await;
    fill_codex(
        credentials,
        provider_tokens,
        credential_sync,
        user_id,
        current_task_id,
    )
    .await;
}

/// The repository a task's git-host token is minted against.
fn task_git_host_repo(repository: Option<&Repository>) -> Option<GitHostRepo> {
    let repository = repository?;
    let provider = repository.provider?;
    GitHostRepo::parse(provider, repository.remote_url.as_deref()?)
}

/// Resolve the git-host tokens this task's sandbox needs.
///
/// `actor` is the identity the task acts as, and it plus the Workspace it names
/// are the only credential authorities: a sandbox gets git-host access through
/// that identity's own rows or its Workspace's pin, never the repository's and
/// never another account's. Holding none leaves the slot empty, and preflight
/// refuses to start the sandbox.
///
/// Each slot already filled is left alone, so a pin that
/// [`resolve_task_credentials`] seeded is not resolved twice.
///
/// `GITHUB_TOKEN` is minted against the task's own repository, so it is filled
/// only for a GitHub-hosted task. `GITLAB_TOKEN` and `GITNESS_TOKEN` are single
/// stored values and are filled whenever the identity holds one, because a
/// workspace may mount repositories from more than one host.
async fn fill_git_host_tokens(
    credentials: &mut HashMap<String, String>,
    git_host_tokens: Option<&Arc<GitHostTokens>>,
    actor: &Actor,
    repository: Option<&Repository>,
) {
    let Some(git_host_tokens) = git_host_tokens else {
        return;
    };

    if !credentials.contains_key(GITHUB_TOKEN) {
        if let Some(resolved) =
            resolve_task_github_token(Some(&Arc::clone(git_host_tokens)), actor, repository).await
        {
            insert_token(credentials, GITHUB_TOKEN, actor, resolved);
        }
    }

    for (host, key) in [
        (PlainGitHost::Gitlab, GITLAB_TOKEN),
        (PlainGitHost::Gitness, GITNESS_TOKEN),
    ] {
        if credentials.contains_key(key) {
            continue;
        }
        insert_token(
            credentials,
            key,
            actor,
            git_host_tokens.host_token(actor, host).await,
        );
    }
}

/// Store a resolved token, or account for why there is none.
///
/// An identity that has simply not connected the host is the ordinary case and
/// is silent — preflight is where that becomes a refusal. Anything else is a
/// credential that exists and does not work, which an operator has to see.
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

/// Resolve `CODEX_AUTH_JSON` for this task.
///
/// Delegates to [`CodexCredentials::ensure_fresh_codex_for`] which owns
/// the `user > global` precedence walk; this function is a thin
/// fallback shim for callers without the orchestrator wired (tests /
/// no-DB) — there we walk the same chain via raw DAO reads with no refresh.
async fn fill_codex(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn HarnessCredentials>,
    credential_sync: Option<&Arc<dyn CodexCredentials>>,
    user_id: &str,
    current_task_id: Option<&str>,
) {
    if credentials.contains_key(CODEX_AUTH_JSON) {
        return;
    }

    if let Some(svc) = credential_sync {
        match svc.ensure_fresh_codex_for(user_id, current_task_id).await {
            Ok(Some(blob)) => {
                credentials.insert(CODEX_AUTH_JSON.to_string(), blob);
            }
            // No codex row at any scope — leave the slot empty and let the
            // scoped-secret fallback try the env-var blob.
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, user_id, "ensure_fresh_codex_for failed");
            }
        }
        return;
    }

    // No CredentialService: walk user > global with raw DAO reads.
    use moatless_types::agent::CodexScope;
    let scopes = [CodexScope::user(user_id), CodexScope::Global];

    for scope in scopes {
        match provider_tokens.codex_blob(&scope).await {
            Ok(Some(blob)) => {
                credentials.insert(CODEX_AUTH_JSON.to_string(), blob);
                return;
            }
            Ok(None) => continue,
            Err(e) => {
                warn!(error = %e, scope = %scope, "Failed to query codex provider_token");
            }
        }
    }
}

/// Resolve the model credentials this task's sandbox needs.
///
/// Git hosts are not here: they go through `GitHostTokens`, which is the one
/// way to a git-host token.
async fn fill_model_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn HarnessCredentials>,
    user_id: &str,
) {
    for (provider, cred_key) in [("openai", OPENAI_API_KEY)] {
        if credentials.contains_key(cred_key) {
            continue;
        }
        match provider_tokens.provider_token(user_id, provider).await {
            Ok(Some(token)) => {
                credentials.insert(cred_key.to_string(), token);
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

    use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
    use moatless_identity::auth::test_support::StubProviderTokens;
    use moatless_identity::secret::test_support::{secret, service_with};

    fn github_repository(scope: moatless_types::access_control::Scope) -> Repository {
        Repository {
            id: "repo-1".to_string(),
            name: "repo".to_string(),
            kind: moatless_types::repository::RepositoryKind::Remote,
            remote_url: Some("https://github.com/acme/repo.git".to_string()),
            default_branch: None,
            description: None,
            provider: Some(moatless_types::repository::RepositoryProvider::Github),
            scope,
            owner_user_id: Some("u1".to_string()),
            created_by: Some("u1".to_string()),
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    /// A seam with no App registry and no OAuth refresh wired — the shape a
    /// deployment without a GitHub App has.
    fn seam(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        secrets: SecretService,
    ) -> Arc<GitHostTokens> {
        Arc::new(GitHostTokens::new(
            provider_tokens,
            Arc::new(moatless_identity::auth::provider_tokens::MockProviderTokenAuthDao::new()),
            None,
            Arc::new(moatless_identity::auth::github::registry::GitHubAppRegistry::default()),
            Arc::new(secrets),
        ))
    }

    /// The `GITHUB_TOKEN` the sandbox would receive, flattened the way
    /// `fill_git_host_tokens` flattens it: any failure leaves the slot empty.
    async fn sandbox_github_token(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        secrets: SecretService,
        repository: &Repository,
    ) -> Option<String> {
        resolve_task_github_token(
            Some(&seam(provider_tokens, secrets)),
            &Actor::of("u1"),
            Some(repository),
        )
        .await
        .and_then(Result::ok)
    }

    /// The stub token store behind the published harness-credential port.
    fn harness(provider_tokens: &Arc<dyn ProviderTokenDao>) -> Arc<dyn HarnessCredentials> {
        Arc::new(moatless_identity::ports::HarnessTokens::new(
            Arc::clone(provider_tokens),
        ))
    }

    /// Inputs for the whole-chain test, with credentials pinned to the task's
    /// workspace.
    fn workspace_github_inputs<'a>(
        actor: &'a Actor,
        provider_tokens: &'a Arc<dyn HarnessCredentials>,
        git_host_tokens: &'a Arc<GitHostTokens>,
        secrets: Option<&'a Arc<SecretService>>,
        repository: &'a Repository,
        workspace_credentials: HashMap<String, String>,
    ) -> TaskCredentialInputs<'a> {
        TaskCredentialInputs {
            task_id: "t1",
            actor,
            secrets,
            provider_tokens: Some(provider_tokens),
            git_host_tokens: Some(git_host_tokens),
            repository: Some(repository),
            users: None,
            credential_sync: None,
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
        let mut resolver = moatless_backend_core::workspace::MockWorkspaceResolver::new();
        resolver.expect_resolve().returning(move |id| {
            Ok(Some(moatless_backend_core::workspace::ResolvedWorkspace {
                id: id.to_string(),
                name: "ws".to_string(),
                docker_image: None,
                docker_access: None,
                sandbox_service_account: None,
                system_prompt: None,
                setup_commands: None,
                env_vars: env_vars.clone(),
                servers: None,
                resources: None,
                network_profile: None,
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
        let workspace_env_vars =
            resolve_workspace_env_vars(Some(&workspaces), Some("ws_1")).await;

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

        assert_eq!(env_vars, HashMap::from([("CARGO_BUILD_JOBS".to_string(), "4".to_string())]));
    }

    /// `fill_git_host_tokens` fills only a slot that is still empty, so a
    /// workspace-declared git-host token would not merely outrank the validated
    /// resolver — it would suppress it, and land in the sandbox as the
    /// clone/push credential.
    #[tokio::test]
    async fn a_workspace_cannot_declare_a_git_host_token_as_plain_config() {
        let workspaces = workspace_resolving_to(
            None,
            Some(HashMap::from([
                (GITHUB_TOKEN.to_string(), "ghp_bypass".to_string()),
                (GITLAB_TOKEN.to_string(), "glpat_bypass".to_string()),
                (GITNESS_TOKEN.to_string(), "gitness_bypass".to_string()),
                // The two the pod derives rather than resolves: `gh` reads
                // GH_TOKEN before `hosts.yml`, and init::git clones with
                // GIT_TOKEN whenever no provider token overwrote it.
                (GH_TOKEN.to_string(), "ghp_bypass".to_string()),
                (GIT_TOKEN.to_string(), "ghp_bypass".to_string()),
                ("CARGO_BUILD_JOBS".to_string(), "4".to_string()),
            ])),
        );

        let env_vars = resolve_workspace_env_vars(Some(&workspaces), Some("ws_1")).await;

        assert_eq!(env_vars, HashMap::from([("CARGO_BUILD_JOBS".to_string(), "4".to_string())]));
    }

    /// And the merge refuses one too, so a caller that assembles
    /// `workspace_env_vars` from somewhere other than the resolver cannot
    /// reopen the bypass. The user here holds a real row: without the guard the
    /// workspace value would occupy the slot before it is ever consulted.
    #[tokio::test]
    async fn a_workspace_env_git_host_token_never_suppresses_the_resolver() {
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row("oauth", "gho_validated"),
        )));
        let repository = github_repository(moatless_types::access_control::Scope::User);
        let git_host_tokens = seam(Arc::clone(&tokens), service_with(vec![]));
        let actor = Actor::of("u1");
        let harness = harness(&tokens);

        let mut inputs = workspace_github_inputs(
            &actor,
            &harness,
            &git_host_tokens,
            None,
            &repository,
            HashMap::new(),
        );
        inputs.workspace_env_vars = HashMap::from([
            (GITHUB_TOKEN.to_string(), "ghp_bypass".to_string()),
            (GH_TOKEN.to_string(), "ghp_bypass".to_string()),
            (GIT_TOKEN.to_string(), "ghp_bypass".to_string()),
        ]);

        let credentials = resolve_task_credentials(inputs).await;

        assert_eq!(credentials[GITHUB_TOKEN], "gho_validated");
        // The pod derives these two from the resolved token; nothing a
        // workspace supplied may be sitting in the slot when it does.
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
                (OPENAI_API_KEY.to_string(), "sk-fine".to_string()),
            ]),
        );

        assert_eq!(
            credentials,
            HashMap::from([(OPENAI_API_KEY.to_string(), "sk-fine".to_string())])
        );
    }

    #[tokio::test]
    async fn no_workspace_yields_no_env_vars() {
        assert!(resolve_workspace_env_vars(None, Some("ws_1")).await.is_empty());
        let workspaces = workspace_resolving_to(None, None);
        assert!(resolve_workspace_env_vars(Some(&workspaces), None).await.is_empty());
        assert!(resolve_workspace_env_vars(Some(&workspaces), Some("ws_1")).await.is_empty());
    }

    #[tokio::test]
    async fn a_workspace_run_as_identity_beats_whoever_started_the_task() {
        // The point of the feature: the workspace names the account, not
        // whoever happened to start the task.
        let workspaces = workspace_running_as(Some("bot"));
        let actor = resolve_task_actor(Some(&workspaces), Some("ws_1"), "starter").await;
        assert_eq!(actor.user_id(), "bot");
    }

    #[tokio::test]
    async fn a_workspace_naming_nobody_leaves_the_task_owner_acting() {
        let workspaces = workspace_running_as(None);
        let actor = resolve_task_actor(Some(&workspaces), Some("ws_1"), "starter").await;
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
            let actor = resolve_task_actor(Some(&workspaces), Some("ws_1"), "starter").await;
            assert_eq!(actor.workspace_id(), Some("ws_1"), "run_as {run_as:?}");
        }
    }

    #[tokio::test]
    async fn an_unreadable_workspace_still_names_itself_on_the_actor() {
        // The pin is looked up from the id alone, so a workspace row that
        // cannot be read narrows the run-as identity without also dropping the
        // credential the workspace pins.
        let actor = resolve_task_actor(None, Some("ws_1"), "starter").await;
        assert_eq!(actor.user_id(), "starter");
        assert_eq!(actor.workspace_id(), Some("ws_1"));
    }

    #[tokio::test]
    async fn a_task_with_no_workspace_acts_as_its_own_owner() {
        let workspaces = workspace_running_as(Some("bot"));
        assert_eq!(
            resolve_task_actor(Some(&workspaces), None, "starter")
                .await
                .user_id(),
            "starter"
        );
        assert_eq!(
            resolve_task_actor(None, Some("ws_1"), "starter")
                .await
                .user_id(),
            "starter"
        );
    }

    #[tokio::test]
    async fn a_workspace_credential_for_another_provider_leaves_github_alone() {
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row("oauth", "gho_from_user"),
        )));
        let repository = github_repository(moatless_types::access_control::Scope::User);
        let pinned = HashMap::from([(OPENAI_API_KEY.to_string(), "sk-workspace".to_string())]);
        let git_host_tokens = seam(Arc::clone(&tokens), service_with(vec![]));
        let actor = Actor::of("u1");
        let harness = harness(&tokens);

        let credentials = resolve_task_credentials(workspace_github_inputs(
            &actor,
            &harness,
            &git_host_tokens,
            None,
            &repository,
            pinned,
        ))
        .await;
        assert_eq!(credentials[GITHUB_TOKEN], "gho_from_user");
        assert_eq!(credentials[OPENAI_API_KEY], "sk-workspace");
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
        let repository = github_repository(moatless_types::access_control::Scope::User);
        let pinned = HashMap::from([(OPENAI_API_KEY.to_string(), "sk-workspace".to_string())]);

        let git_host_tokens = seam(Arc::clone(&tokens), service_with(vec![]));
        let actor = Actor::of("u1");
        let harness = harness(&tokens);
        let credentials = resolve_task_credentials(workspace_github_inputs(
            &actor,
            &harness,
            &git_host_tokens,
            Some(&secrets),
            &repository,
            pinned,
        ))
        .await;

        assert_eq!(credentials[OPENAI_API_KEY], "sk-workspace");
        assert!(
            !credentials.contains_key(GITHUB_TOKEN),
            "this workspace pins no GitHub token and the user holds no row either"
        );
    }

    #[tokio::test]
    async fn the_seam_resolves_the_pin_for_an_actor_naming_the_workspace() {
        // The two channels a pinned token reaches a task by — this map, and
        // `GitHostTokens` for every backend-side call — must not disagree. Here
        // the map is seeded with nothing, so the token can only have come from
        // the seam, via the workspace the actor names.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row("oauth", "gho_from_user"),
        )));
        let repository = github_repository(moatless_types::access_control::Scope::User);
        let mut pins = moatless_backend_core::workspace::MockWorkspaceCredentialReader::new();
        pins.expect_resolve_env().returning(|_| {
            Ok(HashMap::from([(
                GITHUB_TOKEN.to_string(),
                "ghp_workspace".to_string(),
            )]))
        });
        let git_host_tokens = Arc::new(
            GitHostTokens::new(
                Arc::clone(&tokens),
                Arc::new(moatless_identity::auth::provider_tokens::MockProviderTokenAuthDao::new()),
                None,
                Arc::new(moatless_identity::auth::github::registry::GitHubAppRegistry::default()),
                Arc::new(service_with(vec![])),
            )
            .with_workspace_credentials(Arc::new(pins)),
        );
        let actor = Actor::of("u1").on_workspace(Some("ws_1"));
        let harness = harness(&tokens);

        let credentials = resolve_task_credentials(workspace_github_inputs(
            &actor,
            &harness,
            &git_host_tokens,
            None,
            &repository,
            HashMap::new(),
        ))
        .await;

        assert_eq!(credentials[GITHUB_TOKEN], "ghp_workspace");
    }

    #[tokio::test]
    async fn a_workspace_pinned_github_token_is_the_one_the_sandbox_receives() {
        // The pin is the account the workspace names, so it beats the row the
        // user who started the task holds. `fill_git_host_tokens` must leave
        // the seeded slot alone rather than resolve over it.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row("oauth", "gho_from_user"),
        )));
        let repository = github_repository(moatless_types::access_control::Scope::User);
        let pinned = HashMap::from([(GITHUB_TOKEN.to_string(), "ghp_workspace".to_string())]);
        let git_host_tokens = seam(Arc::clone(&tokens), service_with(vec![]));
        let actor = Actor::of("u1");
        let harness = harness(&tokens);

        let credentials = resolve_task_credentials(workspace_github_inputs(
            &actor,
            &harness,
            &git_host_tokens,
            None,
            &repository,
            pinned,
        ))
        .await;

        assert_eq!(credentials[GITHUB_TOKEN], "ghp_workspace");
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
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_validated",
        )]);

        let repository = github_repository(moatless_types::access_control::Scope::User);
        let token = sandbox_github_token(tokens, secrets, &repository).await;
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn env_kind_secret_cannot_bypass_repository_scope() {
        // A plain `env` GITHUB_TOKEN is never injected into a repository
        // sandbox, so preflight and runtime delivery cannot disagree.
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(None));
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "env",
            GITHUB_TOKEN,
            "ghp_unvalidated",
        )]);

        let repository = github_repository(moatless_types::access_control::Scope::User);
        let token = sandbox_github_token(tokens, secrets, &repository).await;
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn provider_tokens_row_still_wins_over_the_secret() {
        let tokens: Arc<dyn ProviderTokenDao> = Arc::new(StubProviderTokens::with_user_row(Some(
            moatless_identity::auth::test_support::github_row("oauth", "gho_from_row"),
        )));
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_from_secret",
        )]);

        let repository = github_repository(moatless_types::access_control::Scope::User);
        let token = sandbox_github_token(tokens, secrets, &repository).await;
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

        let repository = github_repository(moatless_types::access_control::Scope::Global);
        let token = sandbox_github_token(tokens, service_with(vec![]), &repository).await;
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

        let repository = github_repository(moatless_types::access_control::Scope::Global);
        let token = sandbox_github_token(tokens, service_with(vec![]), &repository).await;
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
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_from_secret",
        )]);

        let repository = github_repository(moatless_types::access_control::Scope::User);
        let token = sandbox_github_token(tokens, secrets, &repository).await;
        assert_eq!(token, Some("ghp_from_secret".to_string()));
    }

    // --- GITHUB_TOKEN exact repository scope ---

    #[tokio::test]
    async fn global_github_token_does_not_backstop_user_scope() {
        let tokens: Arc<dyn ProviderTokenDao> =
            Arc::new(StubProviderTokens::default().with_global_row(
                moatless_identity::auth::test_support::github_row("manual", "ghp_org"),
            ));

        let repository = github_repository(moatless_types::access_control::Scope::User);
        let token = sandbox_github_token(tokens, service_with(vec![]), &repository).await;
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
            role: moatless_types::user::UserRole::User,
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
    ) -> Arc<dyn CommitIdentity> {
        use moatless_identity::auth::users::MockUserDao;
        let mut mock = MockUserDao::new();
        mock.expect_get_user()
            .returning(move |_| Ok(Some(make_user(login, name, email, is_bot))));
        Arc::new(moatless_identity::ports::UserDirectory::new(
            Arc::new(mock),
            Arc::new(moatless_identity::auth::test_support::StubProviderTokens::default()),
        ))
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
