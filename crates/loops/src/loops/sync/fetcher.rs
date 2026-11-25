//! GitHub REST API-based fetcher for `.moatless/loops/` directories.
//!
//! Near-copy of the workspace band's skill fetcher: lists `.moatless/loops/`
//! via `GET /repos/{owner}/{repo}/contents/.moatless/loops` and fetches each
//! `LOOP.md` with `Accept: application/vnd.github.raw+json`; a 404 on the
//! parent directory means "no loops" and returns an empty vector.
//!
//! Loops are GitHub-only for v1 (matching skills); GitLab fetching is a
//! follow-up, returning an empty list with a warning.

use async_trait::async_trait;
use futures::future::try_join_all;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::debug;

use super::error::LoopError;
use moatless_types::http::HttpTimeouts;
use moatless_workspace_types::repository::RepositoryProvider;
use moatless_workspace_types::repository::RepositoryRemote;

/// Repo-relative directory that holds committed loops.
const LOOPS_PREFIX: &str = ".moatless/loops";

/// Raw `LOOP.md` content plus its enclosing directory name, so callers can
/// parse frontmatter themselves and use the directory name as the loop's stable
/// key.
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
        remote: &RepositoryRemote,
        token: Option<&str>,
        branch: Option<&str>,
    ) -> Result<Vec<FetchedLoopFile>, LoopError>;
}

/// GitHub REST API implementation of [`LoopFetcher`].
pub struct GitHubLoopFetcher {
    client: Client,
    /// `Config::github_api_base`.
    api_base: String,
}

impl GitHubLoopFetcher {
    /// A fetcher whose client is bounded by `timeouts`.
    ///
    /// Panics when the TLS backend cannot be loaded, as `reqwest::Client::new`
    /// does.
    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub fn new(timeouts: HttpTimeouts, api_base: String) -> Self {
        let client = Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.total)
            .user_agent(moatless_core::USER_AGENT)
            .build()
            .expect("the TLS backend initialises");
        Self { client, api_base }
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
        remote: &RepositoryRemote,
        token: Option<&str>,
        branch: Option<&str>,
    ) -> Result<Vec<FetchedLoopFile>, LoopError> {
        let remote_url = remote.url.as_str();
        let parsed = remote.host_repo.as_ref().ok_or_else(|| LoopError::Fetch {
            git_url: remote_url.to_string(),
            message: "repository has no resolved GitHub host".to_string(),
        })?;
        if parsed.provider() != RepositoryProvider::Github {
            return Ok(Vec::new());
        }
        let (owner, repo) = (parsed.owner().to_string(), parsed.name().to_string());

        let reference =
            branch.map_or_default(|branch| format!("?ref={}", urlencoding::encode(branch)));
        let api_base = self.api_base.as_str();
        let list_url =
            format!("{api_base}/repos/{owner}/{repo}/contents/{LOOPS_PREFIX}{reference}");
        let Some(entries) = send_github_json::<Vec<ContentEntry>>(
            &self.client,
            &list_url,
            "application/vnd.github+json",
            token,
            remote_url,
        )
        .await?
        else {
            // GitHub answers 404 for a repository the credential cannot see
            // too, and an empty answer removes every synced Loop: confirm the
            // repository is visible before reading the 404 as "no Loops".
            let repo_url = format!("{api_base}/repos/{owner}/{repo}");
            let visible = send_github_json::<serde_json::Value>(
                &self.client,
                &repo_url,
                "application/vnd.github+json",
                token,
                remote_url,
            )
            .await?;
            if visible.is_none() {
                return Err(LoopError::Fetch {
                    git_url: remote_url.to_string(),
                    message: "repository not visible with the credential presented".to_string(),
                });
            }
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
                    "{api_base}/repos/{owner}/{repo}/contents/{LOOPS_PREFIX}/{dir_name}/LOOP.md{reference}"
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
/// `Err(LoopError::Fetch)` on any other failure.
async fn send_github_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: &str,
    accept: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<T>, LoopError> {
    let mut req = client
        .get(url)
        .header("User-Agent", moatless_core::USER_AGENT)
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
        .header("User-Agent", moatless_core::USER_AGENT)
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
            _remote: &RepositoryRemote,
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
            .fetch_loop_files(
                &RepositoryRemote {
                    url: "https://github.com/x/y.git".to_string(),
                    host_repo: Some(moatless_workspace_types::repository::GitHostRepo::github(
                        "x", "y",
                    )),
                },
                None,
                Some("main"),
            )
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].dir_name, "java-review");
    }

    #[tokio::test]
    async fn non_github_url_returns_empty() {
        let fetcher = GitHubLoopFetcher::new(
            moatless_core::config::HttpConfig::default().git_content,
            moatless_core::config::DEFAULT_GITHUB_API_BASE.to_string(),
        );
        let files = fetcher
            .fetch_loop_files(
                &RepositoryRemote {
                    url: "https://gitlab.com/foo/bar.git".to_string(),
                    host_repo: Some(moatless_workspace_types::repository::GitHostRepo::new(
                        RepositoryProvider::Gitlab,
                        "gitlab.com",
                        "foo",
                        "bar",
                    )),
                },
                None,
                None,
            )
            .await
            .unwrap();
        assert!(files.is_empty());
    }
}
