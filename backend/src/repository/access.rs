//! Repository-aware provider access and credential selection.

use std::sync::Arc;

use moatless_backend_core::scope::Scope;

use crate::auth::error::AuthError;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::ProviderTokenAuthDao;
use crate::credentials::github::scope::GitHubScope;
use crate::secret::service::SecretService;

use super::config_reader::AccessStatus;
use super::error::RepositoryError;
use super::models::Repository;

/// Select the single persisted GitHub credential scope for a repository.
fn github_credential_scope(repository: &Repository) -> Result<GitHubScope, RepositoryError> {
    match repository.scope {
        Scope::User => repository
            .owner_user_id
            .as_deref()
            .map(GitHubScope::user)
            .ok_or_else(|| {
                RepositoryError::ProviderError(format!(
                    "User-scoped repository '{}' has no owner",
                    repository.id
                ))
            }),
        Scope::Global => Ok(GitHubScope::Global),
    }
}

/// Resolves a repository's GitHub token from the composition-root credential
/// ports, for operations with no run-as identity in play — the repository's
/// persisted scope selects the credential.
pub struct RepositoryTokenResolver {
    provider_tokens: Arc<dyn ProviderTokenDao>,
    provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    github_app_service: Option<Arc<GitHubAppService>>,
    github_apps: Arc<GitHubAppRegistry>,
    secrets: Arc<SecretService>,
}

impl RepositoryTokenResolver {
    pub(crate) fn new(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
        github_app_service: Option<Arc<GitHubAppService>>,
        github_apps: Arc<GitHubAppRegistry>,
        secrets: Arc<SecretService>,
    ) -> Self {
        Self {
            provider_tokens,
            provider_token_auth,
            github_app_service,
            github_apps,
            secrets,
        }
    }

    /// Resolve a GitHub token for an operation on `remote_url`, with no run-as
    /// identity in play — the repository's persisted scope selects the credential.
    pub(crate) async fn resolve(
        &self,
        repository: &Repository,
        remote_url: &str,
    ) -> Result<Option<String>, RepositoryError> {
        resolve_repository_github_token_with(
            self.provider_tokens.as_ref(),
            Some(self.provider_token_auth.as_ref()),
            self.github_app_service.as_deref(),
            self.github_apps.as_ref(),
            Some(self.secrets.as_ref()),
            repository,
            remote_url,
            None,
        )
        .await
    }
}

/// Dependency-injected form used by sandbox credential delivery.
///
/// `run_as_user_id` is the identity the work is attributed to (a trigger's
/// `run_as_user_id`, carried on the task). Naming one makes that user the sole
/// credential authority: their own GitHub row is the only thing consulted, and
/// holding none yields `Ok(None)` rather than borrowing the repository's
/// credential. Callers with no run-as identity fall to the repository's
/// `scope` column as before.
///
/// The strictness is the point — a task must run with the GitHub access its
/// own identity was granted, so a missing or unmintable credential surfaces
/// as "this user cannot reach GitHub" instead of quietly running with someone
/// else's token. `preflight_validate_github_token` turns the `None` into a
/// `NoGithubToken` that names the user and stops the sandbox from starting.
///
/// The URL owner and repository name are passed through to the canonical
/// resolver so Global bindings can validate their installation account and mint
/// a token restricted to this repository.
#[allow(clippy::too_many_arguments)]
// reason: this is the repository credential boundary shared by AppState and gRPC.
pub(crate) async fn resolve_repository_github_token_with(
    provider_tokens: &dyn ProviderTokenDao,
    provider_token_auth: Option<&dyn ProviderTokenAuthDao>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    secrets: Option<&SecretService>,
    repository: &Repository,
    remote_url: &str,
    run_as_user_id: Option<&str>,
) -> Result<Option<String>, RepositoryError> {
    let (repo_owner, repo_name) = parse_github_owner_repo(remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;

    if let Some(user_id) = run_as_user_id {
        return crate::auth::credentials::resolve_run_as_user_github_token(
            provider_tokens,
            provider_token_auth,
            github_app_service,
            github_apps,
            secrets,
            user_id,
            &repo_owner,
            &repo_name,
        )
        .await
        .map_err(|e| {
            RepositoryError::ProviderError(format!(
                "GitHub token resolution failed for run-as user '{user_id}': {e}"
            ))
        });
    }

    let scope = github_credential_scope(repository)?;

    crate::auth::credentials::resolve_repository_github_scope_token(
        provider_tokens,
        provider_token_auth,
        github_app_service,
        github_apps,
        secrets,
        &scope,
        &repo_owner,
        &repo_name,
    )
    .await
    .map_err(repository_token_resolution_error)
}

fn repository_token_resolution_error(error: AuthError) -> RepositoryError {
    match &error {
        AuthError::Forbidden => RepositoryError::Forbidden(
            "GitHub App installation cannot access this repository".to_string(),
        ),
        AuthError::Config(message)
            if message.contains("does not have access to repository")
                || message.contains("does not match bound GitHub installation account") =>
        {
            RepositoryError::Forbidden(message.clone())
        }
        _ => RepositoryError::ProviderError(format!("GitHub token resolution failed: {error}")),
    }
}

/// Parse a supported GitHub remote URL into `(owner, repository)`.
///
/// Both HTTPS/HTTP and the common SCP-style SSH form are accepted. Other hosts,
/// extra path segments, and empty owner/repository names are rejected.
pub(crate) fn parse_github_owner_repo(remote_url: &str) -> Option<(String, String)> {
    let trimmed = remote_url.trim().trim_end_matches('/');
    let path = if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        path.to_string()
    } else {
        let parsed = reqwest::Url::parse(trimmed).ok()?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.host_str()?.eq_ignore_ascii_case("github.com")
        {
            return None;
        }
        parsed.path().trim_start_matches('/').to_string()
    };

    let path = path.trim_end_matches(".git").trim_end_matches('/');
    let mut segments = path.split('/');
    let owner = segments.next()?.trim();
    let repo = segments.next()?.trim();
    if owner.is_empty() || repo.is_empty() || segments.next().is_some() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// Human-readable detail persisted for an access verification failure.
pub(crate) fn access_error_for(status: AccessStatus, url: &str) -> Option<String> {
    match status {
        AccessStatus::Ok | AccessStatus::Unchecked => None,
        AccessStatus::NotFound => Some(format!("Repository not found: {url}")),
        AccessStatus::Forbidden => Some(format!("No access to repository: {url}")),
        AccessStatus::Error => Some(format!("Failed to verify repository: {url}")),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::repository::models::{RepositoryKind, RepositoryProvider};

    fn repository(scope: Scope, owner_user_id: Option<&str>) -> Repository {
        Repository {
            id: "repo-1".to_string(),
            name: "repo".to_string(),
            kind: RepositoryKind::Remote,
            remote_url: Some("https://github.com/acme/repo.git".to_string()),
            default_branch: None,
            description: None,
            provider: Some(RepositoryProvider::Github),
            scope,
            owner_user_id: owner_user_id.map(str::to_string),
            created_by: Some("creator".to_string()),
            access_status: "unchecked".to_string(),
            access_checked_at: None,
            access_error: None,
            deleted: Some(false),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        }
    }

    #[test]
    fn user_scope_selects_repository_owner() {
        let repo = repository(Scope::User, Some("owner"));
        assert_eq!(
            github_credential_scope(&repo).unwrap(),
            GitHubScope::user("owner")
        );
    }

    #[test]
    fn global_scope_selects_global_credential() {
        assert_eq!(
            github_credential_scope(&repository(Scope::Global, None)).unwrap(),
            GitHubScope::Global
        );
    }

    #[test]
    fn malformed_user_scope_without_owner_is_rejected() {
        let repo = repository(Scope::User, None);
        let err = github_credential_scope(&repo)
            .expect_err("ownerless user-scoped row must not use caller credentials");
        assert!(matches!(err, RepositoryError::ProviderError(_)));
    }

    #[tokio::test]
    async fn a_run_as_user_without_a_row_gets_nothing_rather_than_the_repository_credential() {
        // Naming a run-as user makes them the only authority. The same inputs
        // without one still fall to the repository's scope, so the two results
        // differing is what pins the absence of a fallback.
        let tokens = crate::auth::test_support::StubProviderTokens::with_user_row(None)
            .with_global_row(crate::auth::test_support::github_row("oauth", "gho_global"));
        let registry = GitHubAppRegistry::default();
        let repo = repository(Scope::Global, None);
        let url = "https://github.com/acme/repo.git";

        let strict = resolve_repository_github_token_with(
            &tokens,
            None,
            None,
            &registry,
            None,
            &repo,
            url,
            Some("u1"),
        )
        .await
        .expect("a user holding no row is an absent credential, not an error");
        assert_eq!(strict, None);

        let repo_scoped = resolve_repository_github_token_with(
            &tokens, None, None, &registry, None, &repo, url, None,
        )
        .await;
        assert!(
            repo_scoped.is_err(),
            "the repository path reaches the Global row and rejects a non-installation binding"
        );
    }

    #[test]
    fn github_parser_accepts_https_and_ssh_and_rejects_other_hosts() {
        assert_eq!(
            parse_github_owner_repo("https://github.com/acme/repo.git"),
            Some(("acme".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("git@github.com:acme/repo.git"),
            Some(("acme".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://gitlab.com/acme/repo"),
            None
        );
        assert_eq!(parse_github_owner_repo("not-a-url"), None);
    }
}
