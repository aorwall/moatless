//! Pre-provision validation of a task's sandbox credentials.
//!
//! Runs synchronously before lifecycle provisions the sandbox so that a task
//! whose pod could never authenticate fails fast — no Deployment, PVC, or pod
//! is ever created — and the failure is surfaced as a single clear
//! `sandbox.error` event on the task instead of a buried `Sandbox stopped:
//! permanent init failure.` log dump from the pod watcher.
//!
//! Two checks run, in order:
//!
//! 1. **Task control** — the pod authenticates back to the backend as the
//!    task's `created_by`, and the gRPC control gate then requires
//!    `ScopeContext::can_control` on the task row. This evaluates the same
//!    predicate up front so the two cannot drift.
//! 2. **GitHub credentials** — the token the pod will be handed exists and can
//!    see the repository.
//!
//! The GitHub check probes `GET /repos/{owner}/{repo}` directly. Installation
//! tokens are repository credentials and cannot use the user-only `/user`
//! endpoint. The probe runs only when the repo URL is a parseable `github.com`
//! URL.
//!
//! Token resolution mirrors `resolve_task_credentials`: the persisted
//! repository scope selects exactly one credential row. User repositories use
//! the immutable owner's PAT/OAuth credential; Team and Global repositories
//! use only their bound GitHub App installation. Scoped `GITHUB_TOKEN` secrets
//! are never fallback credentials.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use moatless_backend_core::scope::{Scope, ScopeContext};
use moatless_identity::auth::models::UserRole;
use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_identity::auth::github::service::GitHubAppService;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::auth::provider_tokens::ProviderTokenAuthDao;
use moatless_identity::auth::users::UserDao;
use moatless_tasks::credential_sync::CredentialSync;
use moatless_workspaces::repository::access::parse_github_owner_repo;
use moatless_workspaces::repository::models::{Repository, RepositoryKind, RepositoryProvider};
use crate::secrets::{resolve_task_github_token, TaskCredentialInputs};
use moatless_identity::secret::service::SecretService;
use moatless_tasks::task::creation::{PreflightPort, PreflightRejection};
use moatless_tasks::task::models::Task;

/// The exact set of credential-resolution dependencies the preflight consumes.
/// Assembled once at the composition root and consumed by `RealPreflight`,
/// which is held on `AppState` as `preflight` and shared by
/// `TaskCreationService` and the task-start handler. All fields are cheap
/// `Arc` clones.
#[derive(Clone)]
pub struct PreflightCredentials {
    pub secrets: Arc<SecretService>,
    pub provider_tokens: Arc<dyn ProviderTokenDao>,
    pub github_app_service: Option<Arc<GitHubAppService>>,
    pub provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    pub github_apps: Arc<GitHubAppRegistry>,
    pub users: Arc<dyn UserDao>,
    pub credential_sync: Arc<CredentialSync>,
}

/// Failure modes for the pre-flight check.
#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    #[error("user {user_id} cannot control task {task_id} (scope {scope}, owner {owner:?})")]
    NoTaskControl {
        task_id: String,
        user_id: String,
        scope: Scope,
        owner: Option<String>,
    },

    #[error("GitHub account not connected for user {user_id}")]
    NoGithubToken { user_id: String },

    #[error("GitHub token invalid: {reason}")]
    InvalidGithubToken { reason: String },

    #[error("No access to GitHub repository {owner}/{repo}: {reason}")]
    NoRepoAccess {
        owner: String,
        repo: String,
        reason: String,
    },
}

impl PreflightError {
    /// Short user-facing message safe to surface in events and 4xx
    /// responses. Keep distinct per variant so the operator can tell
    /// "token expired" from "token can't see this repo" at a glance.
    pub fn user_message(&self) -> String {
        match self {
            Self::NoTaskControl { user_id, scope, .. } => format!(
                "The sandbox runs as {user_id}, who cannot control this {scope}-scoped task. \
                Recreate the task so it is owned by the user it runs as."
            ),
            Self::NoGithubToken { .. } => "GitHub access is not configured for this repository \
                scope. Configure it before starting this task."
                .to_string(),
            Self::InvalidGithubToken { reason } => {
                format!("GitHub token is invalid or expired: {reason}")
            }
            Self::NoRepoAccess { owner, repo, .. } => format!(
                "GitHub token has no access to repository {owner}/{repo}. \
                Reconnect GitHub or grant the token access to this repo."
            ),
        }
    }
}

/// Abstraction over the two `api.github.com` calls the preflight makes,
/// so unit tests can exercise the logic without hitting the network.
#[async_trait]
pub trait GithubValidator: Send + Sync {
    /// `GET /repos/{owner}/{repo}` — returns `Ok(())` on 2xx.
    async fn check_repo_access(&self, token: &str, owner: &str, repo: &str) -> Result<(), String>;
}

/// Production validator — delegates to the helpers in `github_client`.
pub struct RealGithubValidator;

#[async_trait]
impl GithubValidator for RealGithubValidator {
    async fn check_repo_access(&self, token: &str, owner: &str, repo: &str) -> Result<(), String> {
        moatless_core::github_client::check_github_repo_access(token, owner, repo)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Production preflight — holds exactly the credential handles
/// `preflight_validate_github_token` consumes plus the network validator seam.
///
/// Lives here (sandbox-control) rather than the tasks band so the tasks-band
/// [`PreflightPort`] names no credential or validator type. It maps the rich
/// [`PreflightError`] down to the band-clean [`PreflightRejection`] at the seam,
/// pre-computing the user-facing message and detail the caller needs.
pub struct RealPreflight {
    creds: PreflightCredentials,
    validator: Arc<dyn GithubValidator>,
}

impl RealPreflight {
    pub fn new(creds: PreflightCredentials, validator: Arc<dyn GithubValidator>) -> Self {
        Self { creds, validator }
    }

    /// Both checks, cheapest first, in the rich error type.
    async fn check(&self, task: &Task, repo: &Repository) -> Result<(), PreflightError> {
        preflight_validate_task_control(self.creds.users.as_ref(), task).await?;
        preflight_validate_github_token(&self.creds, task, repo, self.validator.as_ref()).await
    }
}

#[async_trait]
impl PreflightPort for RealPreflight {
    async fn validate(&self, task: &Task, repo: &Repository) -> Result<(), PreflightRejection> {
        self.check(task, repo)
            .await
            .map_err(|e| PreflightRejection {
                user_message: e.user_message(),
                detail: e.to_string(),
            })
    }
}

/// Verify the sandbox's own identity can control the task before provisioning.
///
/// The pod authenticates back as `task.created_by`
/// (`auto_provision::build_provision_request`), and every sandbox gRPC call —
/// `GetCredentials` first among them — is gated on
/// [`ScopeContext::can_control`]. A task that fails this check yields a pod
/// that is refused its credentials, never completes init, fails its startup
/// probe, and crash-loops. Evaluating the identical predicate here turns that
/// into one error before any infrastructure exists.
pub async fn preflight_validate_task_control(
    users: &dyn UserDao,
    task: &Task,
) -> Result<(), PreflightError> {
    let user_id = task.created_by.as_deref().unwrap_or("system");
    let ctx = ScopeContext {
        user_id: user_id.to_string(),
        is_admin: is_platform_admin(users, user_id).await,
    };
    let resource = task.scoped_resource();
    if ctx.can_control(&resource) {
        return Ok(());
    }
    Err(PreflightError::NoTaskControl {
        task_id: task.task_id.clone(),
        user_id: user_id.to_string(),
        scope: resource.scope,
        owner: resource.owner_user_id,
    })
}

/// Platform-admin lookup, mirroring the gRPC control gate: a non-uuid id (the
/// `system` sentinel, a `bot:` value) or a failed read is "not admin".
async fn is_platform_admin(users: &dyn UserDao, user_id: &str) -> bool {
    let Ok(uuid) = user_id.parse::<uuid::Uuid>() else {
        return false;
    };
    matches!(users.get_user(&uuid).await, Ok(Some(u)) if u.role == UserRole::Admin)
}

/// Verify the GitHub token + repo access before provisioning a sandbox.
///
/// `Ok(())` for non-GitHub repos, repos with no remote URL, or non-`github.com`
/// hosted URLs — those cases are deliberately out of scope (handled by other
/// providers or the existing in-pod clone failure path).
pub async fn preflight_validate_github_token(
    creds: &PreflightCredentials,
    task: &Task,
    repo: &Repository,
    validator: &dyn GithubValidator,
) -> Result<(), PreflightError> {
    if !requires_github_preflight(repo) {
        return Ok(());
    }

    let user_id = task.created_by.as_deref().unwrap_or("system").to_string();
    let token = resolve_task_github_token(TaskCredentialInputs {
        task_id: &task.task_id,
        user_id: &user_id,
        // Required to dereference a `secret_ref` row's sentinel; without it
        // every PAT-backed credential resolves as absent.
        secrets: Some(&creds.secrets),
        provider_tokens: Some(&creds.provider_tokens),
        github_app_service: creds.github_app_service.as_ref(),
        provider_token_auth: Some(&creds.provider_token_auth),
        github_apps: Some(&creds.github_apps),
        repository: Some(repo),
        users: Some(&creds.users),
        credential_sync: Some(&creds.credential_sync),
    })
    .await
    .ok_or_else(|| PreflightError::NoGithubToken {
        user_id: user_id.clone(),
    })?;

    let Some(remote_url) = repo.remote_url.as_deref() else {
        return Ok(());
    };

    if !is_github_dot_com(remote_url) {
        warn!(
            remote_url,
            task_id = %task.task_id,
            "Preflight: skipping repo access check for non-github.com URL"
        );
        return Ok(());
    }

    let Some((owner, repo_name)) = parse_github_owner_repo(remote_url) else {
        warn!(
            remote_url,
            task_id = %task.task_id,
            "Preflight: failed to parse GitHub owner/repo from URL"
        );
        return Ok(());
    };

    if let Err(e) = validator
        .check_repo_access(&token, &owner, &repo_name)
        .await
    {
        return Err(PreflightError::NoRepoAccess {
            owner,
            repo: repo_name,
            reason: truncate(&e, 200),
        });
    }

    Ok(())
}

/// Whether a repo's sandbox needs a validated GitHub credential to start.
///
/// Templates are public starter repos the sandbox clones anonymously, so they
/// are exempt even though they carry `provider: Github`.
fn requires_github_preflight(repo: &Repository) -> bool {
    repo.provider == Some(RepositoryProvider::Github) && repo.kind != RepositoryKind::Template
}

fn is_github_dot_com(url: &str) -> bool {
    url.contains("github.com/") || url.contains("github.com:")
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

    /// Mock validator with configurable responses + call counters so tests
    /// can assert which branch was taken.
    struct MockValidator {
        repo_result: std::sync::Mutex<Result<(), String>>,
        repo_calls: AtomicUsize,
    }

    impl Default for MockValidator {
        fn default() -> Self {
            Self {
                repo_result: std::sync::Mutex::new(Ok(())),
                repo_calls: AtomicUsize::new(0),
            }
        }
    }

    impl MockValidator {
        fn set_repo_err(&self, err: &str) {
            *self.repo_result.lock().unwrap() = Err(err.to_string());
        }
    }

    #[async_trait]
    impl GithubValidator for MockValidator {
        async fn check_repo_access(
            &self,
            _token: &str,
            _owner: &str,
            _repo: &str,
        ) -> Result<(), String> {
            self.repo_calls.fetch_add(1, Ordering::SeqCst);
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
            created_at: String::new(),
            last_message_at: String::new(),
            agent_type: moatless_core::shared_models::AgentType("claude-code".to_string()),
            repository_id: "r".to_string(),
            status: moatless_tasks::task::models::TaskStatus::Open,
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
            visibility: moatless_tasks::task::models::Visibility::Private,
            scope,
            owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
            created_at_db: None,
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 0,
            inactive_ttl_days: 0,
            loop_id: None,
            workspace_id: None,
            disabled_servers: None,
            skills: Vec::new(),
        }
    }

    fn user(id: &uuid::Uuid, role: UserRole) -> moatless_identity::auth::users::User {
        moatless_identity::auth::users::User {
            id: *id,
            provider_user_id: None,
            login: id.to_string(),
            email: None,
            name: None,
            avatar_url: None,
            provider: "test".to_string(),
            role,
            created_at: chrono::DateTime::UNIX_EPOCH,
            last_login_at: chrono::DateTime::UNIX_EPOCH,
            settings: serde_json::json!({}),
            is_bot: false,
        }
    }

    /// A `UserDao` that reports one role for every lookup.
    fn users_with_role(role: UserRole) -> moatless_identity::auth::users::MockUserDao {
        let mut users = moatless_identity::auth::users::MockUserDao::new();
        users
            .expect_get_user()
            .returning(move |id| Ok(Some(user(id, role))));
        users
    }

    /// The bug this gate exists for: a global loop firing as a bot yielded a
    /// global, ownerless task. Its pod authenticates as the bot, and the gRPC
    /// gate grants control only to a user-scoped row's owner — so `GetCredentials`
    /// is refused and the pod crash-loops. Reject before any pod exists.
    #[tokio::test]
    async fn a_global_task_cannot_be_controlled_by_the_user_its_sandbox_runs_as() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::Global, Some("bot"), None);

        let err = preflight_validate_task_control(&users, &task)
            .await
            .expect_err("a global task is uncontrollable");

        assert!(matches!(err, PreflightError::NoTaskControl { .. }));
        assert!(err.user_message().contains("cannot control"));
    }

    #[tokio::test]
    async fn a_user_task_owned_by_its_creator_passes() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::User, Some("alice"), Some("alice"));

        assert!(preflight_validate_task_control(&users, &task).await.is_ok());
    }

    /// Ownership falls back to `created_by` for rows predating the
    /// `owner_user_id` column, matching `Task::scoped_resource`.
    #[tokio::test]
    async fn a_user_task_with_only_created_by_passes() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::User, Some("alice"), None);

        assert!(preflight_validate_task_control(&users, &task).await.is_ok());
    }

    /// A user-scoped row owned by somebody else is exactly as unreachable as a
    /// global one — the pod runs as `created_by`.
    #[tokio::test]
    async fn a_user_task_owned_by_someone_else_is_rejected() {
        let users = users_with_role(UserRole::User);
        let task = task(Scope::User, Some("alice"), Some("bob"));

        assert!(preflight_validate_task_control(&users, &task)
            .await
            .is_err());
    }

    /// Platform admin short-circuits `can_control`, so the gate would admit the
    /// pod. Preflight must agree, or it rejects tasks that in fact work.
    #[tokio::test]
    async fn an_admin_creator_controls_even_a_global_task() {
        let users = users_with_role(UserRole::Admin);
        let task = task(Scope::Global, Some("root"), None);

        assert!(preflight_validate_task_control(&users, &task).await.is_ok());
    }

    /// A task with no creator provisions as the `system` sentinel, which is not
    /// a uuid and so never resolves to an admin.
    #[tokio::test]
    async fn a_creatorless_task_is_rejected() {
        let mut users = moatless_identity::auth::users::MockUserDao::new();
        users.expect_get_user().never();
        let task = task(Scope::User, None, None);

        assert!(preflight_validate_task_control(&users, &task)
            .await
            .is_err());
    }

    #[test]
    fn preflight_error_user_message_distinct_per_variant() {
        let no_control = PreflightError::NoTaskControl {
            task_id: "t".to_string(),
            user_id: "u".to_string(),
            scope: Scope::Global,
            owner: None,
        };
        let no_token = PreflightError::NoGithubToken {
            user_id: "u".to_string(),
        };
        let invalid = PreflightError::InvalidGithubToken {
            reason: "401".to_string(),
        };
        let no_access = PreflightError::NoRepoAccess {
            owner: "o".to_string(),
            repo: "r".to_string(),
            reason: "404".to_string(),
        };
        assert!(no_control.user_message().contains("cannot control"));
        assert!(no_token.user_message().contains("not configured"));
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

    /// Minimal repo carrying only the fields the preflight gate reads.
    fn repo(kind: RepositoryKind, provider: Option<RepositoryProvider>) -> Repository {
        Repository {
            id: "r".into(),
            name: "r".into(),
            kind,
            remote_url: Some("https://github.com/o/r".into()),
            default_branch: None,
            description: None,
            provider,
            scope: moatless_api_schemas::enums::Scope::User,
            owner_user_id: None,
            created_by: None,
            access_status: "unchecked".into(),
            access_checked_at: None,
            access_error: None,
            deleted: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn github_remote_repo_requires_preflight() {
        let r = repo(RepositoryKind::Remote, Some(RepositoryProvider::Github));
        assert!(requires_github_preflight(&r));
    }

    #[test]
    fn template_repo_is_exempt_even_when_provider_is_github() {
        // The seeded starter template is a public repo the sandbox clones
        // anonymously — requiring a token would wall off first-run users.
        let r = repo(RepositoryKind::Template, Some(RepositoryProvider::Github));
        assert!(!requires_github_preflight(&r));
    }

    #[test]
    fn non_github_and_provider_less_repos_are_exempt() {
        assert!(!requires_github_preflight(&repo(
            RepositoryKind::Remote,
            Some(RepositoryProvider::Gitlab)
        )));
        assert!(!requires_github_preflight(&repo(
            RepositoryKind::Remote,
            None
        )));
    }

    #[test]
    fn is_github_dot_com_matches_https_and_ssh() {
        assert!(is_github_dot_com("https://github.com/owner/repo"));
        assert!(is_github_dot_com("git@github.com:owner/repo.git"));
        assert!(!is_github_dot_com("https://gitlab.com/owner/repo"));
        assert!(!is_github_dot_com("https://example.com/owner/repo"));
    }

    #[tokio::test]
    async fn validator_checks_repository_directly() {
        let v = MockValidator::default();
        v.set_repo_err("You do not have access...");
        assert!(v.check_repo_access("ghs_x", "o", "r").await.is_err());
        assert_eq!(v.repo_calls.load(Ordering::SeqCst), 1);
    }
}
