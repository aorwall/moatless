//! Repository-aware provider access and credential selection.

use moatless_backend_core::scope::Scope;

use crate::extractors::AuthUser;
use crate::state::AppState;

use super::config_reader::AccessStatus;
use super::error::RepositoryError;
use super::models::Repository;

/// Select the identity whose GitHub credentials apply to a repository.
///
/// User-scoped repositories are immutable-owned resources, so an admin viewing
/// one must still use the owner's credential walk (`owner user > global`).
/// Team/global repositories retain the caller-based walk.
fn github_credential_identity<'a>(
    repository: &'a Repository,
    caller_user_id: &'a str,
    caller_team_ids: &'a [String],
) -> Result<(&'a str, &'a [String]), RepositoryError> {
    match repository.scope {
        Scope::User => repository
            .owner_user_id
            .as_deref()
            .map(|owner_user_id| (owner_user_id, &[][..]))
            .ok_or_else(|| {
                RepositoryError::ProviderError(format!(
                    "User-scoped repository '{}' has no owner",
                    repository.id
                ))
            }),
        Scope::Team | Scope::Global => Ok((caller_user_id, caller_team_ids)),
    }
}

/// Resolve a GitHub token for an operation on `remote_url`.
///
/// The URL owner is passed through to the canonical resolver so bot credentials
/// mint an installation token for the correct GitHub organization.
pub(crate) async fn resolve_repository_github_token(
    state: &AppState,
    repository: &Repository,
    auth: &AuthUser,
    remote_url: &str,
) -> Result<Option<String>, RepositoryError> {
    let (repo_owner, _) = parse_github_owner_repo(remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;
    let caller_user_id = auth.user_id.to_string();
    let (credential_user_id, team_ids) =
        github_credential_identity(repository, &caller_user_id, &auth.team_ids)?;

    crate::auth::credentials::resolve_state_github_token_for(
        state,
        credential_user_id,
        team_ids,
        Some(&repo_owner),
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
            team_id: Some("team-1".to_string()),
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
    fn user_scope_selects_repository_owner_instead_of_caller() {
        let repo = repository(Scope::User, Some("owner"));
        let caller_teams = vec!["admin-team".to_string()];

        let (user_id, team_ids) =
            github_credential_identity(&repo, "viewing-admin", &caller_teams).unwrap();

        assert_eq!(user_id, "owner");
        assert!(
            team_ids.is_empty(),
            "owner walk must fall back directly to global"
        );
    }

    #[test]
    fn team_and_global_scopes_select_caller() {
        let caller_teams = vec!["team-1".to_string()];
        for scope in [Scope::Team, Scope::Global] {
            let repo = repository(scope, None);
            let (user_id, team_ids) =
                github_credential_identity(&repo, "caller", &caller_teams).unwrap();
            assert_eq!(user_id, "caller");
            assert_eq!(team_ids, caller_teams);
        }
    }

    #[test]
    fn malformed_user_scope_without_owner_is_rejected() {
        let repo = repository(Scope::User, None);
        let err = github_credential_identity(&repo, "caller", &[])
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
