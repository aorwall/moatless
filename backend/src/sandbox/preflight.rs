//! Pre-provision validation of GitHub credentials.
//!
//! Runs synchronously before lifecycle provisions the sandbox so that a task
//! with a stale or unauthorised GitHub token fails fast — no Deployment,
//! PVC, or pod is ever created — and the failure is surfaced as a single
//! clear `sandbox.error` event on the task instead of a buried `Sandbox
//! stopped: permanent init failure.` log dump from the pod watcher.
//!
//! The check is a layered probe of `api.github.com`:
//! 1. `GET /user` — distinguishes "token outright invalid" from
//!    "token can't see this repo".
//! 2. `GET /repos/{owner}/{repo}` — only when the repo URL is a
//!    parseable `github.com` URL.
//!
//! Token resolution mirrors `resolve_task_credentials`: team
//! `provider_tokens` → user `provider_tokens` (with GitHub-App refresh),
//! resolving `secret_ref` rows through the `SecretService`. Only a
//! `provider_token`-kind `GITHUB_TOKEN` secret satisfies the check; a plain
//! `env` one stays the operator escape hatch we don't pre-validate.

use async_trait::async_trait;
use tracing::warn;

use crate::repository::handlers::parse_github_owner_repo;
use crate::repository::models::{Repository, RepositoryKind, RepositoryProvider};
use crate::sandbox::secrets::{resolve_task_github_token, TaskCredentialInputs};
use crate::state::AppState;
use crate::task::models::Task;

/// Failure modes for the pre-flight check.
#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
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
            Self::NoGithubToken { .. } => "GitHub account not connected. \
                Connect a GitHub token before starting this task."
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
    /// `GET /user` — returns `Ok(())` on 2xx.
    async fn validate_token(&self, token: &str) -> Result<(), String>;

    /// `GET /repos/{owner}/{repo}` — returns `Ok(())` on 2xx.
    async fn check_repo_access(&self, token: &str, owner: &str, repo: &str) -> Result<(), String>;
}

/// Production validator — delegates to the existing helpers in
/// `provider_settings::github_client`.
pub struct RealGithubValidator;

#[async_trait]
impl GithubValidator for RealGithubValidator {
    async fn validate_token(&self, token: &str) -> Result<(), String> {
        crate::provider_settings::github_client::validate_github_token(token)
            .await
            .map(|_login| ())
    }

    async fn check_repo_access(&self, token: &str, owner: &str, repo: &str) -> Result<(), String> {
        crate::provider_settings::github_client::check_github_repo_access(token, owner, repo).await
    }
}

/// Verify the GitHub token + repo access before provisioning a sandbox.
///
/// `Ok(())` for non-GitHub repos, repos with no remote URL, or non-`github.com`
/// hosted URLs — those cases are deliberately out of scope (handled by other
/// providers or the existing in-pod clone failure path).
pub async fn preflight_validate_github_token(
    state: &AppState,
    task: &Task,
    repo: &Repository,
    validator: &dyn GithubValidator,
) -> Result<(), PreflightError> {
    if !requires_github_preflight(repo) {
        return Ok(());
    }

    let user_id = task.created_by.as_deref().unwrap_or("system").to_string();
    let repo_owner = repo
        .remote_url
        .as_deref()
        .and_then(crate::repository::handlers::parse_github_owner_repo)
        .map(|(owner, _)| owner);

    let token = resolve_task_github_token(TaskCredentialInputs {
        task_id: &task.task_id,
        user_id: &user_id,
        team_id: task.team_id.as_deref(),
        // Required to dereference a `secret_ref` row's sentinel; without it
        // every PAT-backed credential resolves as absent.
        secrets: Some(&state.secrets),
        provider_tokens: Some(&state.provider_tokens),
        github_app_service: state.github_app_service.as_ref(),
        provider_token_auth: Some(&state.provider_token_auth),
        github_apps: Some(&state.github_apps),
        repo_owner: repo_owner.as_deref(),
        users: Some(&state.users),
        credentials_svc: Some(&state.credentials_svc),
    })
    .await
    .ok_or_else(|| PreflightError::NoGithubToken {
        user_id: user_id.clone(),
    })?;

    if let Err(e) = validator.validate_token(&token).await {
        return Err(PreflightError::InvalidGithubToken {
            reason: truncate(&e, 200),
        });
    }

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
        token_result: std::sync::Mutex<Result<(), String>>,
        repo_result: std::sync::Mutex<Result<(), String>>,
        token_calls: AtomicUsize,
        repo_calls: AtomicUsize,
    }

    impl Default for MockValidator {
        fn default() -> Self {
            Self {
                token_result: std::sync::Mutex::new(Ok(())),
                repo_result: std::sync::Mutex::new(Ok(())),
                token_calls: AtomicUsize::new(0),
                repo_calls: AtomicUsize::new(0),
            }
        }
    }

    impl MockValidator {
        fn set_token_err(&self, err: &str) {
            *self.token_result.lock().unwrap() = Err(err.to_string());
        }

        fn set_repo_err(&self, err: &str) {
            *self.repo_result.lock().unwrap() = Err(err.to_string());
        }
    }

    #[async_trait]
    impl GithubValidator for MockValidator {
        async fn validate_token(&self, _token: &str) -> Result<(), String> {
            self.token_calls.fetch_add(1, Ordering::SeqCst);
            self.token_result.lock().unwrap().clone()
        }

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

    #[test]
    fn preflight_error_user_message_distinct_per_variant() {
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
        assert!(no_token.user_message().contains("not connected"));
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
            team_id: None,
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
    async fn validator_returns_invalid_token_on_user_endpoint_failure() {
        let v = MockValidator::default();
        v.set_token_err("GitHub API returned 401: Bad credentials");
        assert!(v.validate_token("ghs_x").await.is_err());
        assert_eq!(v.token_calls.load(Ordering::SeqCst), 1);
        assert_eq!(v.repo_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn validator_returns_no_access_on_repo_endpoint_failure() {
        let v = MockValidator::default();
        v.set_repo_err("You do not have access...");
        assert!(v.validate_token("ghs_x").await.is_ok());
        assert!(v.check_repo_access("ghs_x", "o", "r").await.is_err());
        assert_eq!(v.token_calls.load(Ordering::SeqCst), 1);
        assert_eq!(v.repo_calls.load(Ordering::SeqCst), 1);
    }
}
