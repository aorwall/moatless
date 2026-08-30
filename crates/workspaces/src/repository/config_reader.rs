//! Generic provider file reader.
//!
//! Knows only git-connection concerns: fetching a file's raw bytes from a
//! provider API and probing whether a repo ships a `.moatless/` directory. The
//! `.moatless/` file schemas and the git→workspace sync live in
//! `crate::workspace::git_sync`.

use tracing::{debug, warn};

use super::error::RepositoryError;
use moatless_types::repository::RepositoryProvider;
use moatless_types::repository::RepositoryProvider as GitHostProvider;
use moatless_types::repository::GitHostRepo;

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
    let Some(parsed) = GitHostRepo::parse(GitHostProvider::Github, remote_url) else {
        return true;
    };
    let (owner, repo) = (parsed.owner(), parsed.name());

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
    let parsed = GitHostRepo::parse(GitHostProvider::Github, remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;
    let (owner, repo) = (parsed.owner(), parsed.name());

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

    let body = resp
        .text()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("Failed to read {path} body: {e}")))?;
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

    let body = resp
        .text()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("Failed to read {path} body: {e}")))?;
    Ok(Some(body))
}

/// Parse the `owner/group/project` path out of a gitlab.com URL.
fn gitlab_project_path(remote_url: &str) -> Result<String, RepositoryError> {
    let url_trimmed = remote_url.trim_end_matches('/').trim_end_matches(".git");
    url_trimmed
        .strip_prefix("https://gitlab.com/")
        .or_else(|| url_trimmed.strip_prefix("http://gitlab.com/"))
        .map(str::to_string)
        .ok_or_else(|| {
            RepositoryError::BadRequest(format!("Cannot parse GitLab URL: {remote_url}"))
        })
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
}
