//! GitHub REST API-based fetcher for `.moatless/loops/` directories.
//!
//! Near-copy of the workspace band's skill fetcher: lists `.moatless/loops/` via
//! `GET /repos/{owner}/{repo}/contents/.moatless/loops` and fetches each
//! `LOOP.md` with `Accept: application/vnd.github.raw+json`. A 404 on the
//! parent directory is treated as "no loops" and returns an empty vector.
//!
//! Loops are GitHub-only for v1 (matching skills); GitLab/Gitness fetching is
//! a follow-up and currently returns an empty list with a warning.

use async_trait::async_trait;
use futures::future::try_join_all;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::{debug, warn};

use super::error::LoopError;
use moatless_types::repository::RepositoryProvider;
use moatless_types::repository::GitHostRepo;

/// Repo-relative directory that holds committed loops.
const LOOPS_PREFIX: &str = ".moatless/loops";

/// Raw `LOOP.md` content plus its enclosing directory name, so callers can
/// parse frontmatter themselves and use the directory name as the loop's
/// stable key.
#[derive(Debug, Clone)]
pub struct FetchedLoopFile {
    pub dir_name: String,
    pub content: String,
}

/// Abstraction over the GitHub API call so loop sync can be unit-tested with a
/// hand-rolled stub.
#[async_trait]
pub trait LoopFetcher: Send + Sync {
    async fn fetch_loop_files(
        &self,
        remote_url: &str,
        token: Option<&str>,
        branch: Option<&str>,
    ) -> Result<Vec<FetchedLoopFile>, LoopError>;
}

/// GitHub REST API implementation of [`LoopFetcher`].
pub struct GitHubLoopFetcher {
    client: Client,
}

impl GitHubLoopFetcher {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct ContentEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[async_trait]
impl LoopFetcher for GitHubLoopFetcher {
    async fn fetch_loop_files(
        &self,
        remote_url: &str,
        token: Option<&str>,
        branch: Option<&str>,
    ) -> Result<Vec<FetchedLoopFile>, LoopError> {
        let lower = remote_url.to_ascii_lowercase();
        if !lower.contains("github.com") {
            // GitHub-only for v1; other providers are a follow-up.
            warn!(
                git_url = %remote_url,
                "Loop sync only supports GitHub today — skipping non-GitHub repo"
            );
            return Ok(Vec::new());
        }

        let parsed =
            GitHostRepo::parse(RepositoryProvider::Github, remote_url).ok_or_else(|| {
                LoopError::Fetch {
                    git_url: remote_url.to_string(),
                    message: "cannot parse owner/repo from URL".to_string(),
                }
            })?;
        let (owner, repo) = (parsed.owner().to_string(), parsed.name().to_string());

        let reference = branch
            .map(|branch| format!("?ref={}", urlencoding::encode(branch)))
            .unwrap_or_default();
        let list_url = format!(
            "https://api.github.com/repos/{owner}/{repo}/contents/{LOOPS_PREFIX}{reference}"
        );
        let Some(entries) = send_github_json::<Vec<ContentEntry>>(
            &self.client,
            &list_url,
            "application/vnd.github+json",
            token,
            remote_url,
        )
        .await?
        else {
            debug!(remote_url, "No .moatless/loops directory on remote");
            return Ok(Vec::new());
        };

        let dir_names: Vec<String> = entries
            .into_iter()
            .filter(|e| e.entry_type == "dir")
            .map(|e| e.name)
            .collect();

        let fetches = dir_names.into_iter().map(|dir_name| {
            let owner = owner.clone();
            let repo = repo.clone();
            let remote_url = remote_url.to_string();
            let client = self.client.clone();
            let token = token.map(str::to_string);
            let reference = reference.clone();
            async move {
                let file_url = format!(
                    "https://api.github.com/repos/{owner}/{repo}/contents/{LOOPS_PREFIX}/{dir_name}/LOOP.md{reference}"
                );
                let content =
                    fetch_github_raw(&client, &file_url, token.as_deref(), &remote_url).await?;
                Ok::<FetchedLoopFile, LoopError>(FetchedLoopFile {
                    dir_name,
                    // A directory without LOOP.md is still present. Returning
                    // empty content lets the parser classify it as malformed
                    // without turning it into a confirmed removal.
                    content: content.unwrap_or_default(),
                })
            }
        });

        try_join_all(fetches).await
    }
}

/// GET a GitHub Contents API URL and parse the body as JSON.
///
/// Returns `Ok(None)` on 404, `Err(LoopError::AuthRequired)` on 401/403,
/// `Err(LoopError::Fetch)` on any other non-success status or transport
/// failure.
async fn send_github_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: &str,
    accept: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<T>, LoopError> {
    let mut req = client
        .get(url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", accept);
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }

    let resp = req.send().await.map_err(|e| LoopError::Fetch {
        git_url: git_url.to_string(),
        message: format!("GitHub API request failed: {e}"),
    })?;

    let status = resp.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(LoopError::AuthRequired {
            git_url: git_url.to_string(),
        });
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(LoopError::Fetch {
            git_url: git_url.to_string(),
            message: format!("GitHub API returned {status}: {body}"),
        });
    }

    let parsed = resp.json::<T>().await.map_err(|e| LoopError::Fetch {
        git_url: git_url.to_string(),
        message: format!("failed to decode GitHub JSON response: {e}"),
    })?;
    Ok(Some(parsed))
}

/// GET a GitHub file with `Accept: application/vnd.github.raw+json` and return
/// its body as a string. Returns `Ok(None)` on 404.
async fn fetch_github_raw(
    client: &Client,
    url: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<String>, LoopError> {
    let mut req = client
        .get(url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github.raw+json");
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }

    let resp = req.send().await.map_err(|e| LoopError::Fetch {
        git_url: git_url.to_string(),
        message: format!("GitHub API request failed: {e}"),
    })?;

    let status = resp.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(LoopError::AuthRequired {
            git_url: git_url.to_string(),
        });
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(LoopError::Fetch {
            git_url: git_url.to_string(),
            message: format!("GitHub API returned {status}: {body}"),
        });
    }

    let body = resp.text().await.map_err(|e| LoopError::Fetch {
        git_url: git_url.to_string(),
        message: format!("failed to read GitHub response body: {e}"),
    })?;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubFetcher {
        files: Vec<FetchedLoopFile>,
    }

    #[async_trait]
    impl LoopFetcher for StubFetcher {
        async fn fetch_loop_files(
            &self,
            _remote_url: &str,
            _token: Option<&str>,
            _branch: Option<&str>,
        ) -> Result<Vec<FetchedLoopFile>, LoopError> {
            Ok(self.files.clone())
        }
    }

    #[tokio::test]
    async fn stub_returns_injected_files() {
        let stub = StubFetcher {
            files: vec![FetchedLoopFile {
                dir_name: "java-review".into(),
                content: "---\non: github_pr\n---\n".into(),
            }],
        };
        let files = stub
            .fetch_loop_files("https://github.com/x/y.git", None, Some("main"))
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].dir_name, "java-review");
    }

    #[tokio::test]
    async fn non_github_url_returns_empty() {
        let fetcher = GitHubLoopFetcher::new(Client::new());
        let files = fetcher
            .fetch_loop_files("https://gitlab.com/foo/bar.git", None, None)
            .await
            .unwrap();
        assert!(files.is_empty());
    }
}
