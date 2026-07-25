//! Repository-aware provider access and credential selection.

use moatless_backend_core::scope::Scope;

use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::credentials::github::scope::GitHubScope;
use crate::secret::service::SecretService;
use crate::state::AppState;

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

/// Resolve a GitHub token for an operation on `remote_url`.
///
/// The URL owner and repository name are passed through to the canonical
/// resolver so Team/Global bindings can validate their installation account
/// and mint a token restricted to this repository.
pub(crate) async fn resolve_repository_github_token(
    state: &AppState,
    repository: &Repository,
    remote_url: &str,
) -> Result<Option<String>, RepositoryError> {
    resolve_repository_github_token_with(
        state.provider_tokens.as_ref(),
        Some(state.provider_token_auth.as_ref()),
        state.github_app_service.as_deref(),
        state.github_apps.as_ref(),
        Some(state.secrets.as_ref()),
        repository,
        remote_url,
    )
    .await
}

/// Dependency-injected form used by sandbox credential delivery.
#[allow(clippy::too_many_arguments)]
// reason: this is the repository credential boundary shared by AppState and gRPC.
pub(crate) async fn resolve_repository_github_token_with(
    provider_tokens: &dyn ProviderTokenRepository,
    provider_token_auth: Option<&dyn ProviderTokenAuthRepository>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    secrets: Option<&SecretService>,
    repository: &Repository,
    remote_url: &str,
) -> Result<Option<String>, RepositoryError> {
    let (repo_owner, repo_name) = parse_github_owner_repo(remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;
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
    .map_err(|e| RepositoryError::ProviderError(format!("GitHub token resolution failed: {e}")))
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
