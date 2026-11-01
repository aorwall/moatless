//! Pre-provision validation of a task's sandbox credentials.
//!
//! Runs before provisioning, so a pod that could never authenticate fails fast
//! as one `sandbox.error`, not buried in the pod watcher's log. Checks task
//! write access, then asks the repository's own host whether the resolved
//! credential can read it. Token resolution goes through
//! [`resolve_task_git_token`], the same function `GitProxy::resolve` answers a
//! proxied clone with, so a credential the preflight accepts is the one the
//! clone presents.

use std::sync::Arc;

use async_trait::async_trait;

use crate::lifecycle::dao::SandboxShape;
use crate::lifecycle::ingress::SandboxIngress;
use crate::secrets::{resolve_task_actor, resolve_task_git_token, resolve_workspace_credentials};
use moatless_ports::identity::CommitIdentity;
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_ports::sandbox::{CodexCredentials, PreflightPort, PreflightRejection};
use moatless_types::access_control::{Scope, ScopeContext, ScopedResource};
use moatless_types::http::HttpTimeouts;
use moatless_types::task::{Task, Visibility};
use moatless_workspace_ports::{WorkspaceCredentialReader, WorkspaceResolver};
use moatless_workspace_types::repository::RepositoryProvider as GitHostProvider;
use moatless_workspace_types::repository::{GitHostRepo, Repository};
use moatless_workspace_types::repository::{RepositoryKind, RepositoryProvider};

/// The exact set of credential-resolution dependencies the preflight consumes.
///
/// Assembled once at the composition root; held on `AppState` as `preflight`
/// and shared by `TaskCreationService` and the task-start handler. All fields
/// are cheap `Arc` clones.
#[derive(Clone)]
pub struct PreflightCredentials {
    /// The one way to a git-host token.
    pub git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    pub users: Arc<dyn CommitIdentity>,
    /// Read for the Workspace's run-as identity, which outranks the task's own
    /// owner — see `secrets::resolve_task_actor`.
    pub workspaces: Arc<dyn WorkspaceResolver>,
    pub credential_sync: Arc<dyn CodexCredentials>,
    /// Provider credentials pinned to the task's Workspace. Read here as well
    /// as at runtime so preflight validates the token the sandbox will actually
    /// be handed, not the one it would have had without the override.
    pub workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
}

/// Failure modes for the pre-flight check.
#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    #[error("user {user_id} cannot write task {task_id} (scope {scope}, owner {owner:?})")]
    NoTaskControl {
        task_id: String,
        user_id: String,
        scope: Scope,
        owner: Option<String>,
    },

    #[error("{provider} account not connected for user {user_id}{}", host.as_deref().map_or(String::new(), |h| format!(" at {h}")))]
    NoGitCredential {
        user_id: String,
        provider: RepositoryProvider,
        /// Which instance of a self-hostable `provider` is unconnected, and
        /// `None` for a repository whose remote names no host this seam can
        /// parse — the other way to reach this variant.
        host: Option<String>,
    },

    #[error("workspace {workspace_id} credentials could not be read: {reason}")]
    WorkspaceCredentialsUnreadable {
        workspace_id: String,
        reason: String,
    },

    #[error("{provider} token invalid: {reason}")]
    InvalidGitCredential {
        provider: RepositoryProvider,
        reason: String,
    },

    #[error("No access to {provider} repository {owner}/{repo} on {host}: {reason}")]
    NoRepoAccess {
        provider: RepositoryProvider,
        host: String,
        owner: String,
        repo: String,
        reason: String,
    },
}

impl PreflightError {
    /// Short user-facing message safe to surface in events and 4xx responses.
    /// Keep distinct per variant so the operator can tell "token expired" from
    /// "token can't see this repo" at a glance.
    pub fn user_message(&self) -> String {
        match self {
            Self::NoTaskControl { user_id, scope, .. } => format!(
                "The sandbox runs as {user_id}, who cannot write this {scope}-scoped task. \
                Recreate the task so it is owned by the user it runs as."
            ),
            Self::NoGitCredential {
                provider,
                host: Some(host),
                ..
            } => format!(
                "{provider} access to {host} is not configured for this repository scope. \
                Configure it before starting this task."
            ),
            Self::NoGitCredential { provider, .. } => format!(
                "{provider} access is not configured for this repository scope. \
                Configure it before starting this task."
            ),
            Self::WorkspaceCredentialsUnreadable { .. } => {
                "This workspace pins its own provider credentials and they could not be read. \
                Re-set them before starting this task."
                    .to_string()
            }
            Self::InvalidGitCredential { provider, reason } => {
                format!("{provider} token is invalid or expired: {reason}")
            }
            Self::NoRepoAccess {
                provider,
                host,
                owner,
                repo,
                ..
            } => format!(
                "The {provider} token has no access to repository {owner}/{repo} on {host}. \
                Reconnect {provider} or grant the token access to this repo."
            ),
        }
    }
}

/// Abstraction over the git-host call the preflight makes, so unit tests can
/// exercise the logic without hitting the network.
#[async_trait]
pub trait GitHostValidator: Send + Sync {
    /// Whether `token` can read `repo` on the host its remote named — GitHub's
    /// `GET /repos/{owner}/{repo}`, Forgejo's `GET /api/v1/repos/{owner}/{repo}`.
    /// `Ok(())` on 2xx.
    async fn check_repo_access(&self, token: &str, repo: &GitHostRepo) -> Result<(), String>;
}

/// Production validator — delegates to the per-provider clients in
/// `moatless_core`.
pub struct RealGitHostValidator {
    /// GitHub REST API root, from `Config::github_api_base`.
    pub github_api_base: String,
    /// From `Config::http`.
    pub github: HttpTimeouts,
    /// From `Config::http`.
    pub self_hosted_git: HttpTimeouts,
}

#[async_trait]
impl GitHostValidator for RealGitHostValidator {
    async fn check_repo_access(&self, token: &str, repo: &GitHostRepo) -> Result<(), String> {
        match repo.provider() {
            GitHostProvider::Github => {
                moatless_core::github_client::check_github_repo_access(
                    self.github,
                    &self.github_api_base,
                    token,
                    repo.owner(),
                    repo.name(),
                )
                .await
            }
            GitHostProvider::Forgejo => {
                moatless_core::forgejo_client::check_forgejo_repo_access(
                    self.self_hosted_git,
                    repo.host(),
                    token,
                    repo.owner(),
                    repo.name(),
                )
                .await
            }
            // No probe is defined for a provider `requires_credential_preflight`
            // exempts: resolving its credential is the whole check.
            GitHostProvider::Gitlab => Ok(()),
        }
        .map_err(|e| e.to_string())
    }
}

/// Holds the credential handles and network validator seam
/// `preflight_validate_git_credential` needs.
///
/// So [`PreflightPort`] names no credential/validator; maps [`PreflightError`]
/// to [`PreflightRejection`], message and detail precomputed.
pub struct RealPreflight {
    creds: PreflightCredentials,
    validator: Arc<dyn GitHostValidator>,
    /// Read for the Sandbox's driver, whom the pod acts for once set.
    sandboxes: Arc<SandboxIngress>,
}

impl RealPreflight {
    pub fn new(
        creds: PreflightCredentials,
        validator: Arc<dyn GitHostValidator>,
        sandboxes: Arc<SandboxIngress>,
    ) -> Self {
        Self {
            creds,
            validator,
            sandboxes,
        }
    }

    /// Both checks, cheapest first, in the rich error type.
    async fn check(&self, task: &Task, repo: Option<&Repository>) -> Result<(), PreflightError> {
        let driver = self.driver(task).await;
        let driver = driver.as_deref();
        preflight_validate_task_control(self.creds.users.as_ref(), task, driver).await?;
        let Some(repo) = repo else {
            return Ok(());
        };
        preflight_validate_git_credential(&self.creds, task, driver, repo, self.validator.as_ref())
            .await
    }

    /// The Sandbox's driver, `None` before its first, before its row exists,
    /// and when the row cannot be read: provisioning re-checks the row itself.
    async fn driver(&self, task: &Task) -> Option<String> {
        let sandbox_id = task.sandbox_id.as_deref().unwrap_or(&task.task_id);
        match self.sandboxes.identity(sandbox_id).await {
            Ok(identity) => identity.and_then(|identity| identity.acting_user_id),
            Err(e) => {
                tracing::warn!(sandbox_id, error = %e, "Preflight could not read the sandbox driver");
                None
            }
        }
    }
}

#[async_trait]
impl PreflightPort for RealPreflight {
    async fn validate<'a>(
        &self,
        task: &Task,
        repo: Option<&'a Repository>,
    ) -> Result<(), PreflightRejection> {
        self.check(task, repo)
            .await
            .map_err(|e| PreflightRejection {
                user_message: e.user_message(),
                detail: e.to_string(),
            })
    }
}

/// Verify the sandbox's own identity can write the task before provisioning.
///
/// The pod authenticates back as `driver`, else the task's creator; every
/// sandbox gRPC call is gated on that user having write, so failing here is
/// what stands between that and a pod that crash-loops forever refused its
/// credentials.
pub async fn preflight_validate_task_control(
    users: &dyn CommitIdentity,
    task: &Task,
    driver: Option<&str>,
) -> Result<(), PreflightError> {
    let user_id = driver.or(task.created_by.as_deref()).unwrap_or("system");
    validate_control(
        users,
        &task.task_id,
        user_id,
        &task.scoped_resource(),
        task.visibility,
    )
    .await
}

/// The same check against the Sandbox, which is where provisioning reads it.
///
/// The pod authenticates back as [`SandboxShape::acting_user_id`]
/// (`auto_provision::build_provision_request`), so that is the identity checked
/// here — not `run_as_user_id`, which is what the pod acts as outward.
pub(crate) async fn preflight_validate_sandbox_control(
    users: &dyn CommitIdentity,
    sandbox_id: &str,
    shape: &SandboxShape,
) -> Result<(), PreflightError> {
    let user_id = shape.acting_user_id().unwrap_or("system");
    let resource = ScopedResource::borrowed(shape.scope, shape.owner_user_id.as_deref());
    let visibility = if shape.public {
        Visibility::Public
    } else {
        Visibility::Private
    };
    validate_control(users, sandbox_id, user_id, &resource, visibility).await
}

async fn validate_control(
    users: &dyn CommitIdentity,
    task_id: &str,
    user_id: &str,
    resource: &ScopedResource,
    visibility: Visibility,
) -> Result<(), PreflightError> {
    let ctx = ScopeContext::new(user_id.to_string(), is_platform_admin(users, user_id).await);
    if ctx.can_write(resource, Some(visibility)) {
        return Ok(());
    }
    Err(PreflightError::NoTaskControl {
        task_id: task_id.to_string(),
        user_id: user_id.to_string(),
        scope: resource.scope,
        owner: resource.owner_user_id.clone(),
    })
}

/// Platform-admin lookup, mirroring the gRPC control gate: no user (`system`,
/// `bot:`) or a failed read is "not admin".
///
/// The port keeps those two cases apart; the control gate deliberately does
/// not, since neither grants admin either way.
async fn is_platform_admin(users: &dyn CommitIdentity, user_id: &str) -> bool {
    users.is_platform_admin(user_id).await.unwrap_or(false)
}

/// Verify the git credential, then the repo access, before provisioning.
///
/// `Ok(())` for a repository whose provider `requires_credential_preflight`
/// exempts. Every other repository must resolve a credential *and* have the
/// host confirm that credential reaches it, so neither half is left to the
/// in-pod clone.
pub async fn preflight_validate_git_credential(
    creds: &PreflightCredentials,
    task: &Task,
    driver: Option<&str>,
    repo: &Repository,
    validator: &dyn GitHostValidator,
) -> Result<(), PreflightError> {
    let Some(provider) = requires_credential_preflight(repo) else {
        return Ok(());
    };
    let host_repo = repo
        .remote
        .as_ref()
        .and_then(|remote| remote.host_repo.as_ref());

    let actor = resolve_task_actor(
        Some(&creds.workspaces),
        task.workspace_id.as_deref(),
        task.run_as_user_id.as_deref(),
        driver.or(task.created_by.as_deref()).unwrap_or("system"),
    )
    .await;
    let user_id = actor.user_id().to_string();
    // Read only to prove it can be read: a workspace whose pinned credentials
    // cannot be decrypted would start a sandbox missing them, which is a
    // different account's access by omission.
    resolve_workspace_credentials(
        Some(&creds.workspace_credentials),
        task.workspace_id.as_deref(),
    )
    .await
    .map_err(|e| PreflightError::WorkspaceCredentialsUnreadable {
        workspace_id: task.workspace_id.clone().unwrap_or_default(),
        reason: truncate(&e.to_string(), 200),
    })?;

    let token = match resolve_task_git_token(Some(&creds.git_host_tokens), &actor, Some(repo)).await
    {
        Some(Ok(token)) => token,
        // The identity has not connected this host, or the repository names no
        // remote this seam can reach. Either way there is no token, and the
        // sandbox must not start with somebody else's.
        Some(Err(GitHostTokenError::NoCredential { .. })) | None => {
            return Err(PreflightError::NoGitCredential {
                user_id,
                provider,
                host: host_repo.map(|repo| repo.host().to_string()),
            });
        }
        // A credential that exists and does not work is a different problem
        // with a different remedy, so it keeps the resolver's own message.
        Some(Err(e)) => {
            return Err(PreflightError::InvalidGitCredential {
                provider,
                reason: truncate(&e.to_string(), 200),
            });
        }
    };

    // Unreachable with `None`: a repository naming no host repo resolves no
    // token above.
    let Some(parsed) = host_repo else {
        return Ok(());
    };

    if let Err(e) = validator.check_repo_access(&token, parsed).await {
        return Err(PreflightError::NoRepoAccess {
            provider: parsed.provider(),
            host: parsed.host().to_string(),
            owner: parsed.owner().to_string(),
            repo: parsed.name().to_string(),
            reason: truncate(&e, 200),
        });
    }

    Ok(())
}

/// The provider whose credential a repo's sandbox must hold to start, if any.
///
/// GitLab is absent — a check that cannot fail is worse than none;
/// templates are exempt despite `provider: Github` since the sandbox clones
/// them anonymously.
fn requires_credential_preflight(repo: &Repository) -> Option<RepositoryProvider> {
    if repo.kind == RepositoryKind::Template {
        return None;
    }
    match repo.provider {
        Some(provider @ (RepositoryProvider::Github | RepositoryProvider::Forgejo)) => {
            Some(provider)
        }
        _ => None,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    format!("{}…", &s[..idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    use moatless_ports::identity::MockGitHostTokenIssuer;
    use moatless_types::git_host::GitHostAccount;

    /// Mock validator with configurable responses + call counters so tests can
    /// assert which branch was taken, and which repository was probed.
    struct MockValidator {
        repo_result: std::sync::Mutex<Result<(), String>>,
        repo_calls: AtomicUsize,
        probed: std::sync::Mutex<Option<GitHostRepo>>,
    }

    impl Default for MockValidator {
        fn default() -> Self {
            Self {
                repo_result: std::sync::Mutex::new(Ok(())),
                repo_calls: AtomicUsize::new(0),
                probed: std::sync::Mutex::new(None),
            }
        }
    }

    impl MockValidator {
        fn set_repo_err(&self, err: &str) {
            *self.repo_result.lock().unwrap() = Err(err.to_string());
        }

        fn calls(&self) -> usize {
            self.repo_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl GitHostValidator for MockValidator {
        async fn check_repo_access(&self, _token: &str, repo: &GitHostRepo) -> Result<(), String> {
            self.repo_calls.fetch_add(1, Ordering::SeqCst);
            *self.probed.lock().unwrap() = Some(repo.clone());
            self.repo_result.lock().unwrap().clone()
        }
    }

    /// Deterministic uuid per name, so a fixture's id is stable and readable.
    fn uid(name: &str) -> uuid::Uuid {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    }

    /// A `Task` fixture carrying only the columns the control gate reads.
    fn task(scope: Scope, created_by: Option<&str>, owner_user_id: Option<&str>) -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: chrono::Utc::now(),
            last_message_at: chrono::Utc::now(),
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            repository_id: Some("r".to_string()),
            status: moatless_types::task::TaskStatus::Open,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: None,
            vibe: None,
            name: None,
            description: None,
            site_id: None,
            site_provider: None,
            branch: None,
            model: None,
            created_by: created_by.map(|u| uid(u).to_string()),
            visibility: moatless_types::task::Visibility::Private,
            scope,
            owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
            run_as_user_id: None,
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 0,
            close_after_settled_days: 0,
            settle_after_hours: None,
            loop_id: None,
            workspace_id: None,
            sandbox_id: None,
            checkout_mode: None,
            memory_preload: Vec::new(),
            skills: Vec::new(),
        }
    }

    use moatless_types::user::UserRole;

    /// A `CommitIdentity` that reports one role for every lookup.
    fn users_with_role(role: UserRole) -> impl CommitIdentity {
        let mut users = moatless_ports::identity::MockCommitIdentity::new();
        users
            .expect_is_platform_admin()
            .returning(move |_| Ok(role == UserRole::Admin));
        users
    }

    /// A `CommitIdentity` that reports an unresolvable actor — what the seam
    /// answers for the `system` sentinel, which is not a user id.
    fn users_rejecting_the_actor() -> impl CommitIdentity {
        let mut users = moatless_ports::identity::MockCommitIdentity::new();
        users.expect_is_platform_admin().returning(|id| {
            Err(moatless_types::store::StoreError::new(format!(
                "'{id}' is not a user id"
            )))
        });
        users
    }

    /// The bug this gate exists for: a global loop firing as a bot yielded a
    /// global, ownerless task, whose pod then authenticates as the bot.
    ///
    /// The gRPC gate grants control only to a user-scoped owner; refused
    /// `GetCredentials` crash-loops the pod, so reject before one exists.
    #[tokio::test]
    async fn a_global_task_cannot_be_controlled_by_the_user_its_sandbox_runs_as() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::Global, Some("bot"), None);

        let err = preflight_validate_task_control(&users, &task, None)
            .await
            .expect_err("a global task is uncontrollable");

        assert!(matches!(err, PreflightError::NoTaskControl { .. }));
        assert!(err.user_message().contains("cannot write"));
    }

    #[tokio::test]
    async fn a_user_task_owned_by_its_creator_passes() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::User, Some("alice"), Some("alice"));

        assert!(
            preflight_validate_task_control(&users, &task, None)
                .await
                .is_ok()
        );
    }

    /// Ownership falls back to `created_by` for rows predating the
    /// `owner_user_id` column, matching `Task::scoped_resource`.
    #[tokio::test]
    async fn a_user_task_with_only_created_by_passes() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::User, Some("alice"), None);

        assert!(
            preflight_validate_task_control(&users, &task, None)
                .await
                .is_ok()
        );
    }

    /// A user-scoped row owned by somebody else is exactly as unreachable as a
    /// global one — the pod runs as `created_by`.
    #[tokio::test]
    async fn a_user_task_owned_by_someone_else_is_rejected() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::User, Some("alice"), Some("bob"));

        assert!(
            preflight_validate_task_control(&users, &task, None)
                .await
                .is_err()
        );
    }

    /// Platform admin short-circuits `is_owner`, so the gate would admit the
    /// pod. Preflight must agree, or it rejects tasks that in fact work.
    #[tokio::test]
    async fn an_admin_creator_controls_even_a_global_task() {
        let users = users_with_role(UserRole::Admin);
        let task = task(Scope::Global, Some("root"), None);

        assert!(
            preflight_validate_task_control(&users, &task, None)
                .await
                .is_ok()
        );
    }

    /// A public task grants write to every signed-in user, so its driver
    /// controls the pod however it is scoped.
    #[tokio::test]
    async fn the_driver_of_a_public_task_passes() {
        let users = users_with_role(UserRole::User);
        let mut task = task(Scope::User, Some("alice"), Some("alice"));
        task.visibility = moatless_types::task::Visibility::Public;
        let bob = uid("bob").to_string();

        assert!(
            preflight_validate_task_control(&users, &task, Some(&bob))
                .await
                .is_ok()
        );
    }

    /// The pod authenticates as the driver, not the owner, once one is set.
    #[tokio::test]
    async fn a_driver_without_write_on_a_private_task_is_rejected() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::User, Some("alice"), Some("alice"));
        let bob = uid("bob").to_string();

        assert!(
            preflight_validate_task_control(&users, &task, Some(&bob))
                .await
                .is_err()
        );
    }

    /// A task with no creator provisions as the `system` sentinel, which is not
    /// a uuid and so never resolves to an admin.
    #[tokio::test]
    async fn a_creatorless_task_is_rejected() {
        let users = users_rejecting_the_actor();
        let task = task(Scope::User, None, None);

        assert!(
            preflight_validate_task_control(&users, &task, None)
                .await
                .is_err()
        );
    }

    #[test]
    fn preflight_error_user_message_distinct_per_variant() {
        let no_control = PreflightError::NoTaskControl {
            task_id: "t".to_string(),
            user_id: "u".to_string(),
            scope: Scope::Global,
            owner: None,
        };
        let no_token = PreflightError::NoGitCredential {
            user_id: "u".to_string(),
            provider: RepositoryProvider::Forgejo,
            host: Some("git.example.com".to_string()),
        };
        let invalid = PreflightError::InvalidGitCredential {
            provider: RepositoryProvider::Github,
            reason: "401".to_string(),
        };
        let no_access = PreflightError::NoRepoAccess {
            provider: RepositoryProvider::Github,
            host: "github.com".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            reason: "404".to_string(),
        };
        assert!(no_control.user_message().contains("cannot write"));
        assert!(no_token.user_message().contains("not configured"));
        assert!(no_token.user_message().contains("git.example.com"));
        assert!(invalid.user_message().contains("invalid or expired"));
        assert!(no_access.user_message().contains("o/r"));
    }

    #[test]
    fn truncate_is_char_boundary_safe() {
        let s = "ä".repeat(150);
        let out = truncate(&s, 50);
        // Each ä is 2 bytes — truncation must land on a char boundary, not
        // mid-codepoint.
        assert!(out.len() <= 50 + "…".len());
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_returns_input_when_short() {
        assert_eq!(truncate("short", 100), "short");
    }

    /// The host a fixture's provider is reached on. Self-hosted providers carry
    /// whatever their remote named, which is the fact the Forgejo cases turn on.
    fn host_of(provider: RepositoryProvider) -> &'static str {
        match provider {
            RepositoryProvider::Github => "github.com",
            _ => "git.example.com",
        }
    }

    /// Minimal repo carrying only the fields the preflight gate reads.
    ///
    /// The remote is parsed for the row's own provider, as
    /// `workspaces::repository::remote::resolve_remote` does — a row naming no
    /// provider has no host repo either.
    fn repo(kind: RepositoryKind, provider: Option<RepositoryProvider>) -> Repository {
        let host = provider.map_or("github.com", host_of);
        Repository {
            id: "r".into(),
            name: "r".into(),
            kind,
            remote: Some(moatless_workspace_types::repository::RepositoryRemote {
                url: format!("https://{host}/o/r"),
                host_repo: provider.map(|provider| GitHostRepo::new(provider, host, "o", "r")),
            }),
            default_branch: None,
            description: None,
            provider,
            scope: moatless_types::access_control::Scope::User,
            owner_user_id: None,
            created_by: None,
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn github_and_forgejo_remote_repos_require_preflight() {
        for provider in [RepositoryProvider::Github, RepositoryProvider::Forgejo] {
            assert_eq!(
                requires_credential_preflight(&repo(RepositoryKind::Remote, Some(provider))),
                Some(provider)
            );
        }
    }

    #[test]
    fn template_repo_is_exempt_even_when_provider_is_github() {
        // The seeded starter template is a public repo the sandbox clones
        // anonymously — requiring a token would wall off first-run users.
        let r = repo(RepositoryKind::Template, Some(RepositoryProvider::Github));
        assert!(requires_credential_preflight(&r).is_none());
    }

    #[test]
    fn unvalidated_and_provider_less_repos_are_exempt() {
        // GitLab has no validated preflight path, so a check here could only
        // ever pass.
        for provider in [Some(RepositoryProvider::Gitlab), None] {
            assert!(
                requires_credential_preflight(&repo(RepositoryKind::Remote, provider)).is_none()
            );
        }
    }

    /// Never reached: the git-credential preflight reads no Codex credential.
    struct NoCodexCredentials;

    #[async_trait]
    impl CodexCredentials for NoCodexCredentials {
        async fn ensure_fresh_codex_for(&self, _user_id: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    /// The preflight's dependencies over `tokens`, with the rest inert: the task
    /// fixture names no Workspace, so nothing below is consulted and a mock that
    /// is called at all fails the test.
    fn creds(tokens: MockGitHostTokenIssuer) -> PreflightCredentials {
        PreflightCredentials {
            git_host_tokens: Arc::new(tokens),
            users: Arc::new(moatless_ports::identity::MockCommitIdentity::new()),
            workspaces: Arc::new(moatless_workspace_ports::MockWorkspaceResolver::new()),
            credential_sync: Arc::new(NoCodexCredentials),
            workspace_credentials: Arc::new(
                moatless_workspace_ports::MockWorkspaceCredentialReader::new(),
            ),
        }
    }

    /// An issuer holding a credential for `held` and nothing else, recording
    /// every account it was asked for.
    fn issuer_for(
        held: Option<RepositoryProvider>,
    ) -> (
        MockGitHostTokenIssuer,
        Arc<std::sync::Mutex<Vec<GitHostAccount>>>,
    ) {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = asked.clone();
        let mut tokens = MockGitHostTokenIssuer::new();
        tokens.expect_token().returning(move |actor, account| {
            recorded.lock().unwrap().push(account.clone());
            if held == Some(account.provider()) {
                return Ok("a-token".to_string());
            }
            Err(GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider: account.provider(),
                host: match account {
                    GitHostAccount::Forgejo { host } => Some(host.clone()),
                    _ => None,
                },
            })
        });
        (tokens, asked)
    }

    fn alice() -> Task {
        task(Scope::User, Some("alice"), Some("alice"))
    }

    /// The failure this gate exists for on a self-hosted host: with no
    /// credential for the instance, the clone can only fail in-pod through the
    /// git proxy, minutes and two pod provisions later.
    #[tokio::test]
    async fn a_forgejo_repository_with_no_forgejo_credential_is_rejected() {
        let (tokens, _) = issuer_for(None);
        let validator = MockValidator::default();

        let err = preflight_validate_git_credential(
            &creds(tokens),
            &alice(),
            None,
            &repo(RepositoryKind::Remote, Some(RepositoryProvider::Forgejo)),
            &validator,
        )
        .await
        .expect_err("a Forgejo repo with no Forgejo credential cannot be cloned");

        assert!(matches!(
            err,
            PreflightError::NoGitCredential {
                provider: RepositoryProvider::Forgejo,
                ..
            }
        ));
        assert!(err.user_message().contains("git.example.com"));
        // There is nothing to probe with, so the host is never called.
        assert_eq!(validator.calls(), 0);
    }

    /// A credential is per account, so the GitHub row an identity does hold is
    /// not an answer for a Forgejo instance it has not connected.
    #[tokio::test]
    async fn a_credential_for_another_provider_does_not_satisfy_a_forgejo_repository() {
        let (tokens, _) = issuer_for(Some(RepositoryProvider::Github));
        let validator = MockValidator::default();

        let err = preflight_validate_git_credential(
            &creds(tokens),
            &alice(),
            None,
            &repo(RepositoryKind::Remote, Some(RepositoryProvider::Forgejo)),
            &validator,
        )
        .await
        .expect_err("a GitHub credential reaches no Forgejo instance");

        assert!(matches!(
            err,
            PreflightError::NoGitCredential {
                provider: RepositoryProvider::Forgejo,
                ..
            }
        ));
        assert_eq!(validator.calls(), 0);
    }

    /// Pins the preflight to the account `GitProxy::resolve` presents for the
    /// same repository. Both resolve through
    /// [`crate::secrets::resolve_task_git_token`]; this is what would break if
    /// either grew a lookup of its own.
    #[tokio::test]
    async fn the_account_resolved_is_the_one_the_git_proxy_presents() {
        for provider in [RepositoryProvider::Github, RepositoryProvider::Forgejo] {
            let (tokens, asked) = issuer_for(Some(provider));
            let repository = repo(RepositoryKind::Remote, Some(provider));

            preflight_validate_git_credential(
                &creds(tokens),
                &alice(),
                None,
                &repository,
                &MockValidator::default(),
            )
            .await
            .expect("the identity holds this provider's credential");

            let host_repo = repository
                .remote
                .as_ref()
                .and_then(|remote| remote.host_repo.as_ref())
                .expect("the fixture parses a host repo");
            assert_eq!(
                asked.lock().unwrap().as_slice(),
                [host_repo.credential_account()]
            );
        }
    }

    /// The second half of the check, on both probed hosts: a credential that
    /// exists and cannot reach this repository is refused before a pod exists.
    #[tokio::test]
    async fn a_repository_the_credential_cannot_reach_is_rejected() {
        for provider in [RepositoryProvider::Github, RepositoryProvider::Forgejo] {
            let (tokens, _) = issuer_for(Some(provider));
            let validator = MockValidator::default();
            validator.set_repo_err("404 Not Found");

            let err = preflight_validate_git_credential(
                &creds(tokens),
                &alice(),
                None,
                &repo(RepositoryKind::Remote, Some(provider)),
                &validator,
            )
            .await
            .expect_err("the host refused the probe");

            assert!(matches!(
                &err,
                PreflightError::NoRepoAccess { provider: probed, host, .. }
                    if *probed == provider && host == host_of(provider)
            ));
            assert_eq!(validator.calls(), 1);
        }
    }

    #[tokio::test]
    async fn a_forgejo_repository_the_credential_reaches_passes() {
        let (tokens, _) = issuer_for(Some(RepositoryProvider::Forgejo));
        let validator = MockValidator::default();

        preflight_validate_git_credential(
            &creds(tokens),
            &alice(),
            None,
            &repo(RepositoryKind::Remote, Some(RepositoryProvider::Forgejo)),
            &validator,
        )
        .await
        .expect("a connected instance that answers for the repo provisions");

        assert_eq!(validator.calls(), 1);
    }
}
