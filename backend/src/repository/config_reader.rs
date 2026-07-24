//! Generic provider file reader + repository access checks.
//!
//! Knows only git-connection concerns: fetching a file's raw bytes from a
//! provider API, probing whether a repo ships a `.moatless/` directory, and
//! verifying a repository URL is reachable. The `.moatless/` file schemas and
//! the git→workspace sync live in `crate::workspace::git_sync`.

use tracing::{debug, warn};

use super::access::parse_github_owner_repo;
use super::error::RepositoryError;
use super::models::RepositoryProvider;

/// Provider reachability of a repository URL. Stored on the `repositories` row
/// as `access_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessStatus {
    Unchecked,
    Ok,
    NotFound,
    Forbidden,
    Error,
}

impl AccessStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccessStatus::Unchecked => "unchecked",
            AccessStatus::Ok => "ok",
            AccessStatus::NotFound => "not_found",
            AccessStatus::Forbidden => "forbidden",
            AccessStatus::Error => "error",
        }
    }
}

/// Fetch a repository file's raw contents via the provider API.
///
/// Returns `Ok(None)` on a 404 (file absent). `path` is a repo-relative path
/// such as `.moatless/config.json`. Gitness is not wired (needs a base_url).
pub(crate) async fn fetch_provider_file(
    remote_url: &str,
    path: &str,
    branch: Option<&str>,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> Result<Option<String>, RepositoryError> {
    match provider {
        RepositoryProvider::Github => fetch_github_file(remote_url, path, branch, token).await,
        RepositoryProvider::Gitlab => {
            let token = token.ok_or_else(|| {
                RepositoryError::BadRequest(
                    "GitLab requires an access token to fetch config".to_string(),
                )
            })?;
            fetch_gitlab_file(remote_url, path, branch, token).await
        }
        RepositoryProvider::Gitness => {
            // Gitness needs a base_url; skip for now – callers can pass it later.
            Ok(None)
        }
    }
}

/// Cheap best-effort check for whether a repo ships a `.moatless/` directory.
///
/// Used to gate the automatic config sync on task start so we don't hit the
/// provider API for repos that don't use moatless config at all. Fails open:
/// on a definitive 404 it returns `false`, but on any other error (transport,
/// rate limit, unsupported provider) it returns `true` so the caller still
/// attempts the sync rather than silently skipping a repo that does have config.
pub async fn moatless_dir_exists(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> bool {
    match provider {
        RepositoryProvider::Github => github_moatless_dir_exists(remote_url, branch, token).await,
        // Non-GitHub providers have no cheap check wired yet — don't skip them.
        _ => true,
    }
}

/// GitHub existence probe: `GET /repos/{owner}/{repo}/contents/.moatless?ref={branch}`.
/// Only a 404 is treated as "no directory"; everything else fails open to `true`.
async fn github_moatless_dir_exists(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
) -> bool {
    let Some((owner, repo)) = parse_github_owner_repo(remote_url) else {
        return true;
    };

    let mut url = format!("https://api.github.com/repos/{owner}/{repo}/contents/.moatless");
    if let Some(r) = branch {
        use std::fmt::Write;
        let _ = write!(url, "?ref={r}");
    }

    let client = reqwest::Client::new();
    let mut req = client
        .get(&url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github+json");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    match req.send().await {
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => false,
        Ok(_) => true,
        Err(e) => {
            warn!(error = %e, "Failed to probe .moatless directory; assuming present");
            true
        }
    }
}

/// Fetch a file from GitHub: `GET /repos/{owner}/{repo}/contents/{path}?ref={branch}`.
/// Returns the raw file body, or `Ok(None)` on 404.
async fn fetch_github_file(
    remote_url: &str,
    path: &str,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<Option<String>, RepositoryError> {
    let (owner, repo) = parse_github_owner_repo(remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;

    let mut url = format!("https://api.github.com/repos/{owner}/{repo}/contents/{path}");
    if let Some(r) = branch {
        use std::fmt::Write;
        let _ = write!(url, "?ref={r}");
    }

    let client = reqwest::Client::new();
    let mut req = client
        .get(&url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github.raw+json");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitHub API request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        debug!("No {path} found in {owner}/{repo}");
        return Ok(None);
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "GitHub API returned {status} fetching {path}: {body}"
        )));
    }

    let body = resp.text().await.map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to read {path} body: {e}"))
    })?;
    Ok(Some(body))
}

/// Fetch a file from GitLab:
/// `GET /api/v4/projects/{id}/repository/files/{encoded_path}/raw?ref={branch}`.
/// Returns the raw file body, or `Ok(None)` on 404.
async fn fetch_gitlab_file(
    remote_url: &str,
    path: &str,
    branch: Option<&str>,
    token: &str,
) -> Result<Option<String>, RepositoryError> {
    let project_path = gitlab_project_path(remote_url)?;

    let encoded_path = urlencoding::encode(&project_path);
    let encoded_file = urlencoding::encode(path);
    let ref_param = branch.unwrap_or("HEAD");

    let api_url = format!(
        "https://gitlab.com/api/v4/projects/{encoded_path}/repository/files/{encoded_file}/raw?ref={ref_param}"
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&api_url)
        .header("PRIVATE-TOKEN", token)
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitLab API request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        debug!("No {path} found in {project_path}");
        return Ok(None);
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "GitLab API returned {status} fetching {path}: {body}"
        )));
    }

    let body = resp.text().await.map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to read {path} body: {e}"))
    })?;
    Ok(Some(body))
}

/// Parse the `owner/group/project` path out of a gitlab.com URL.
fn gitlab_project_path(remote_url: &str) -> Result<String, RepositoryError> {
    let url_trimmed = remote_url.trim_end_matches('/').trim_end_matches(".git");
    url_trimmed
        .strip_prefix("https://gitlab.com/")
        .or_else(|| url_trimmed.strip_prefix("http://gitlab.com/"))
        .map(str::to_string)
        .ok_or_else(|| RepositoryError::BadRequest(format!("Cannot parse GitLab URL: {remote_url}")))
}

/// Validate that a persisted remote URL can be checked by the wired provider
/// verifier. This keeps malformed/unsupported inputs as request errors instead
/// of silently returning `unchecked`.
pub(crate) fn validate_repository_access_target(
    remote_url: &str,
    provider: RepositoryProvider,
) -> Result<(), RepositoryError> {
    match provider {
        RepositoryProvider::Github => parse_github_owner_repo(remote_url)
            .map(|_| ())
            .ok_or_else(|| {
                RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
            }),
        RepositoryProvider::Gitlab => {
            let path = gitlab_project_path(remote_url)?;
            if path.split('/').filter(|part| !part.is_empty()).count() < 2 {
                return Err(RepositoryError::BadRequest(format!(
                    "Cannot parse GitLab URL: {remote_url}"
                )));
            }
            Ok(())
        }
        RepositoryProvider::Gitness => Err(RepositoryError::BadRequest(
            "Gitness access verification is not supported".to_string(),
        )),
    }
}

/// Verify a repository URL is reachable via the provider API, mapping the
/// response to an [`AccessStatus`]: 200→`ok`, 404→`not_found`,
/// 401/403→`forbidden`, anything else (incl. transport errors)→`error`.
///
/// Providers without a wired check (Gitness) and unparseable URLs return
/// `Unchecked`, leaving the repo's default `unchecked` state untouched.
pub async fn verify_repository_access(
    remote_url: &str,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> AccessStatus {
    let url = match provider {
        RepositoryProvider::Github => {
            let Some((owner, repo)) = parse_github_owner_repo(remote_url) else {
                return AccessStatus::Unchecked;
            };
            format!("https://api.github.com/repos/{owner}/{repo}")
        }
        RepositoryProvider::Gitlab => match gitlab_project_path(remote_url) {
            Ok(path) => format!(
                "https://gitlab.com/api/v4/projects/{}",
                urlencoding::encode(&path)
            ),
            Err(_) => return AccessStatus::Unchecked,
        },
        RepositoryProvider::Gitness => return AccessStatus::Unchecked,
    };

    let client = reqwest::Client::new();
    let mut req = client.get(&url).header("User-Agent", "moatless-vibe-backend");
    req = match provider {
        RepositoryProvider::Github => {
            let mut r = req.header("Accept", "application/vnd.github+json");
            if let Some(token) = token {
                r = r.header("Authorization", format!("Bearer {token}"));
            }
            r
        }
        RepositoryProvider::Gitlab => match token {
            Some(token) => req.header("PRIVATE-TOKEN", token),
            None => req,
        },
        RepositoryProvider::Gitness => req,
    };

    match req.send().await {
        Ok(resp) => map_status_to_access(resp.status()),
        Err(e) => {
            warn!(error = %e, %url, "verify_repository_access request failed");
            AccessStatus::Error
        }
    }
}

/// Map an HTTP status to an [`AccessStatus`].
fn map_status_to_access(status: reqwest::StatusCode) -> AccessStatus {
    use reqwest::StatusCode;
    if status.is_success() {
        AccessStatus::Ok
    } else if status == StatusCode::NOT_FOUND {
        AccessStatus::NotFound
    } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        AccessStatus::Forbidden
    } else {
        AccessStatus::Error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn moatless_dir_exists_fails_open_for_non_github() {
        // Non-GitHub providers have no cheap probe wired — the gate must not
        // skip them, so the check reports the directory as present.
        assert!(
            moatless_dir_exists(
                "https://gitness.example.com/x/y.git",
                None,
                None,
                RepositoryProvider::Gitness
            )
            .await
        );
    }

    #[test]
    fn verify_status_mapping() {
        use reqwest::StatusCode;
        assert_eq!(map_status_to_access(StatusCode::OK), AccessStatus::Ok);
        assert_eq!(
            map_status_to_access(StatusCode::NOT_FOUND),
            AccessStatus::NotFound
        );
        assert_eq!(
            map_status_to_access(StatusCode::UNAUTHORIZED),
            AccessStatus::Forbidden
        );
        assert_eq!(
            map_status_to_access(StatusCode::FORBIDDEN),
            AccessStatus::Forbidden
        );
        assert_eq!(
            map_status_to_access(StatusCode::INTERNAL_SERVER_ERROR),
            AccessStatus::Error
        );
        assert_eq!(
            map_status_to_access(StatusCode::TOO_MANY_REQUESTS),
            AccessStatus::Error
        );
    }

    #[test]
    fn access_status_as_str_values() {
        assert_eq!(AccessStatus::Unchecked.as_str(), "unchecked");
        assert_eq!(AccessStatus::Ok.as_str(), "ok");
        assert_eq!(AccessStatus::NotFound.as_str(), "not_found");
        assert_eq!(AccessStatus::Forbidden.as_str(), "forbidden");
        assert_eq!(AccessStatus::Error.as_str(), "error");
    }

    #[test]
    fn access_target_validation_rejects_missing_or_unsupported_provider_urls() {
        assert!(validate_repository_access_target(
            "https://github.com/acme/repo",
            RepositoryProvider::Github
        )
        .is_ok());
        assert!(validate_repository_access_target(
            "https://gitlab.com/group/project",
            RepositoryProvider::Gitlab
        )
        .is_ok());
        assert!(validate_repository_access_target(
            "https://example.com/acme/repo",
            RepositoryProvider::Github
        )
        .is_err());
        assert!(validate_repository_access_target(
            "https://gitness.example/acme/repo",
            RepositoryProvider::Gitness
        )
        .is_err());
    }
}
