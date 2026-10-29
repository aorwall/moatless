//! Generic provider file reader.
//!
//! Knows only git-connection concerns: fetching a file's raw bytes from a
//! provider API and probing whether a repo ships a `.moatless/` directory. The
//! `.moatless/` file schemas and the git→workspace sync live in
//! `crate::workspace::git_sync`.

use tracing::{debug, warn};

use super::error::RepositoryError;
use crate::repository::remote::require_host_repo;
use moatless_types::http::HttpTimeouts;
use moatless_workspace_types::repository::RepositoryProvider;
use moatless_workspace_types::repository::RepositoryRemote;

/// The client every read here takes, each one small file or directory probe.
///
/// Panics when the TLS backend cannot be loaded, as `reqwest::Client::new`
/// does.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
pub(crate) fn client(timeouts: HttpTimeouts) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.total)
        .build()
        .expect("the TLS backend initialises")
}

/// Fetch a repository file's raw contents via the provider API.
///
/// Returns `Ok(None)` on a 404 (file absent).
pub(crate) async fn fetch_provider_file(
    http: &reqwest::Client,
    github_api_base: &str,
    remote: &RepositoryRemote,
    path: &str,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<Option<String>, RepositoryError> {
    let provider = require_host_repo(remote)?.provider();
    match provider {
        RepositoryProvider::Github => {
            fetch_github_file(http, github_api_base, remote, path, branch, token).await
        }
        RepositoryProvider::Gitlab => {
            let token = token.ok_or_else(|| {
                RepositoryError::BadRequest(
                    "GitLab requires an access token to fetch config".to_string(),
                )
            })?;
            fetch_gitlab_file(http, remote, path, branch, token).await
        }
        RepositoryProvider::Forgejo => fetch_forgejo_file(http, remote, path, branch, token).await,
    }
}

/// Cheap best-effort check for whether a repo ships a `.moatless/` directory.
///
/// Gates the automatic config sync on task start. Fails open: only a definitive
/// 404 returns `false`; any other error returns `true` so the caller still
/// attempts the sync rather than silently skipping a repo that has config.
pub async fn moatless_dir_exists(
    http: &reqwest::Client,
    github_api_base: &str,
    remote: &RepositoryRemote,
    branch: Option<&str>,
    token: Option<&str>,
) -> bool {
    let Some(host) = remote.host_repo.as_ref() else {
        return true;
    };
    match host.provider() {
        RepositoryProvider::Github => {
            github_moatless_dir_exists(http, github_api_base, remote, branch, token).await
        }
        RepositoryProvider::Forgejo => {
            forgejo_moatless_dir_exists(http, remote, branch, token).await
        }
        // GitLab has no cheap check wired — don't skip it.
        RepositoryProvider::Gitlab => true,
    }
}

/// GitHub existence probe: `GET
/// /repos/{owner}/{repo}/contents/.moatless?ref={branch}`. Only a 404 is
/// treated as "no directory"; everything else fails open to `true`.
async fn github_moatless_dir_exists(
    http: &reqwest::Client,
    github_api_base: &str,
    remote: &RepositoryRemote,
    branch: Option<&str>,
    token: Option<&str>,
) -> bool {
    let Some(parsed) = remote.host_repo.as_ref() else {
        return true;
    };
    let (owner, repo) = (parsed.owner(), parsed.name());

    let mut url = format!("{github_api_base}/repos/{owner}/{repo}/contents/.moatless");
    if let Some(r) = branch {
        use std::fmt::Write;
        let _ = write!(url, "?ref={r}");
    }

    let client = http;
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

/// Forgejo existence probe: `GET
/// /api/v1/repos/{owner}/{repo}/contents/.moatless?ref={branch}`. Only a 404 is
/// treated as "no directory"; everything else fails open to `true`.
async fn forgejo_moatless_dir_exists(
    http: &reqwest::Client,
    remote: &RepositoryRemote,
    branch: Option<&str>,
    token: Option<&str>,
) -> bool {
    let Ok(url) = forgejo_api_url(remote, "contents/.moatless", branch) else {
        return true;
    };
    let mut req = http.get(&url);
    if let Some(token) = token {
        req = req.header("Authorization", format!("token {token}"));
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

/// Fetch a file from Forgejo: `GET
/// /api/v1/repos/{owner}/{repo}/raw/{path}?ref={branch}`. Returns the raw file
/// body, or `Ok(None)` on 404.
async fn fetch_forgejo_file(
    http: &reqwest::Client,
    remote: &RepositoryRemote,
    path: &str,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<Option<String>, RepositoryError> {
    let remote_url = remote.url.as_str();
    let url = forgejo_api_url(remote, &format!("raw/{path}"), branch)?;
    let mut req = http.get(&url);
    if let Some(token) = token {
        req = req.header("Authorization", format!("token {token}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("Forgejo API request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        debug!("No {path} found in {remote_url}");
        return Ok(None);
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "Forgejo API returned {status} fetching {path}: {body}"
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("Failed to read {path} body: {e}")))?;
    Ok(Some(body))
}

/// A Forgejo REST URL for `suffix` under the repository `remote_url` names.
///
/// HTTPS always, whatever scheme `remote_url` carries — a Workspace controls
/// that URL, and honouring its scheme would let one downgrade the connection
/// the token below travels over.
fn forgejo_api_url(
    remote: &RepositoryRemote,
    suffix: &str,
    branch: Option<&str>,
) -> Result<String, RepositoryError> {
    let parsed = require_host_repo(remote)?;
    let host = parsed.host();
    let (owner, repo) = (parsed.owner(), parsed.name());
    let mut url = format!("https://{host}/api/v1/repos/{owner}/{repo}/{suffix}");
    if let Some(r) = branch {
        use std::fmt::Write;
        let _ = write!(url, "?ref={}", urlencoding::encode(r));
    }
    Ok(url)
}

/// The commit sha `branch` (the default branch when `None`) points at on
/// GitHub: `GET /repos/{owner}/{repo}/commits/{ref}`.
///
/// A 404 is an error, not an absence: GitHub answers 404 for a private
/// repository the credential cannot see, so it proves nothing about the branch.
pub(crate) async fn github_branch_head(
    http: &reqwest::Client,
    github_api_base: &str,
    remote: &RepositoryRemote,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<String, RepositoryError> {
    let parsed = require_host_repo(remote)?;
    let (owner, repo) = (parsed.owner(), parsed.name());
    let reference = urlencoding::encode(branch.unwrap_or("HEAD"));
    let url = format!("{github_api_base}/repos/{owner}/{repo}/commits/{reference}");

    let mut req = http
        .get(&url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github.sha");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitHub API request failed: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(RepositoryError::ProviderError(format!(
            "GitHub API returned {status} reading the head of {owner}/{repo}"
        )));
    }
    let sha = resp
        .text()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("Failed to read head sha: {e}")))?;
    Ok(sha.trim().to_string())
}

/// Fetch a file from GitHub: `GET
/// /repos/{owner}/{repo}/contents/{path}?ref={branch}`. Returns the raw file
/// body, or `Ok(None)` on 404.
async fn fetch_github_file(
    http: &reqwest::Client,
    github_api_base: &str,
    remote: &RepositoryRemote,
    path: &str,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<Option<String>, RepositoryError> {
    let parsed = require_host_repo(remote)?;
    let (owner, repo) = (parsed.owner(), parsed.name());

    let mut url = format!("{github_api_base}/repos/{owner}/{repo}/contents/{path}");
    if let Some(r) = branch {
        use std::fmt::Write;
        let _ = write!(url, "?ref={r}");
    }

    let client = http;
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

/// Fetch a file from GitLab: `GET
/// /api/v4/projects/{id}/repository/files/{encoded_path}/raw?ref={branch}`.
/// Returns the raw file body, or `Ok(None)` on 404.
async fn fetch_gitlab_file(
    http: &reqwest::Client,
    remote: &RepositoryRemote,
    path: &str,
    branch: Option<&str>,
    token: &str,
) -> Result<Option<String>, RepositoryError> {
    let remote_url = remote.url.as_str();
    let parsed = require_host_repo(remote)?;
    if parsed.host() != "gitlab.com" {
        return Err(RepositoryError::BadRequest(format!(
            "Cannot fetch GitLab config from {remote_url}"
        )));
    }
    let project_path = parsed.to_string();

    let encoded_path = urlencoding::encode(&project_path);
    let encoded_file = urlencoding::encode(path);
    let ref_param = branch.unwrap_or("HEAD");

    let api_url = format!(
        "https://gitlab.com/api/v4/projects/{encoded_path}/repository/files/{encoded_file}/raw?ref={ref_param}"
    );

    let client = http;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn moatless_dir_exists_fails_open_for_non_github() {
        // Non-GitHub providers have no cheap probe wired — the gate must not
        // skip them, so the check reports the directory as present.
        assert!(
            moatless_dir_exists(
                &client(moatless_core::config::HttpConfig::default().git_content),
                moatless_core::config::DEFAULT_GITHUB_API_BASE,
                &crate::repository::remote::resolve_remote(
                    Some(RepositoryProvider::Gitlab),
                    "https://gitlab.example.com/x/y.git".to_string()
                ),
                None,
                None,
            )
            .await
        );
    }
}
