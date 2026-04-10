//! GitHub REST API-based fetcher for `.claude/skills/` directories.
//!
//! Uses `GET /repos/{owner}/{repo}/contents/.claude/skills` to list skill
//! directories and `GET /repos/{owner}/{repo}/contents/.claude/skills/{dir}/SKILL.md`
//! with `Accept: application/vnd.github.raw+json` to fetch each SKILL.md body.
//! A 404 on the parent directory is treated as "no skills" and returns an
//! empty vector.

use async_trait::async_trait;
use futures::future::try_join_all;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::debug;

use super::error::SkillError;
use crate::repository::handlers::parse_github_owner_repo;

/// Raw SKILL.md content plus its enclosing directory name, so callers can
/// parse frontmatter themselves and fall back to the directory name when
/// `name:` is missing.
#[derive(Debug, Clone)]
pub struct FetchedSkillFile {
    pub dir_name: String,
    pub content: String,
}

/// Abstraction over the GitHub API call so the orchestrating service can be
/// unit-tested with a hand-rolled stub.
#[async_trait]
pub trait SkillFetcher: Send + Sync {
    async fn fetch_skill_files(
        &self,
        remote_url: &str,
        token: Option<&str>,
    ) -> Result<Vec<FetchedSkillFile>, SkillError>;
}

/// GitHub REST API implementation of [`SkillFetcher`].
pub struct GitHubSkillFetcher {
    client: Client,
}

impl GitHubSkillFetcher {
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
impl SkillFetcher for GitHubSkillFetcher {
    async fn fetch_skill_files(
        &self,
        remote_url: &str,
        token: Option<&str>,
    ) -> Result<Vec<FetchedSkillFile>, SkillError> {
        let lower = remote_url.to_ascii_lowercase();
        if !lower.contains("github.com") {
            return Err(SkillError::UnsupportedProvider {
                git_url: remote_url.to_string(),
            });
        }

        let (owner, repo) = parse_github_owner_repo(remote_url).ok_or_else(|| {
            SkillError::Fetch {
                git_url: remote_url.to_string(),
                message: "cannot parse owner/repo from URL".to_string(),
            }
        })?;

        let list_url = format!(
            "https://api.github.com/repos/{owner}/{repo}/contents/.claude/skills"
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
            debug!(remote_url, "No .claude/skills directory on remote");
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
            async move {
                let file_url = format!(
                    "https://api.github.com/repos/{owner}/{repo}/contents/.claude/skills/{dir_name}/SKILL.md"
                );
                let content = fetch_github_raw(
                    &client,
                    &file_url,
                    token.as_deref(),
                    &remote_url,
                )
                .await?;
                Ok::<Option<FetchedSkillFile>, SkillError>(
                    content.map(|content| FetchedSkillFile { dir_name, content }),
                )
            }
        });

        let results: Vec<Option<FetchedSkillFile>> = try_join_all(fetches).await?;
        Ok(results.into_iter().flatten().collect())
    }
}

/// GET a GitHub Contents API URL and parse the body as JSON.
///
/// Returns `Ok(None)` on 404, `Err(SkillError::AuthRequired)` on 401/403,
/// `Err(SkillError::Fetch)` on any other non-success status or transport
/// failure.
async fn send_github_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: &str,
    accept: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<T>, SkillError> {
    let mut req = client
        .get(url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", accept);
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }

    let resp = req.send().await.map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("GitHub API request failed: {e}"),
    })?;

    let status = resp.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(SkillError::AuthRequired {
            git_url: git_url.to_string(),
        });
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(SkillError::Fetch {
            git_url: git_url.to_string(),
            message: format!("GitHub API returned {status}: {body}"),
        });
    }

    let parsed = resp.json::<T>().await.map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("failed to decode GitHub JSON response: {e}"),
    })?;
    Ok(Some(parsed))
}

/// GET a GitHub file with `Accept: application/vnd.github.raw+json` and
/// return its body as a string. Returns `Ok(None)` on 404.
async fn fetch_github_raw(
    client: &Client,
    url: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<String>, SkillError> {
    let mut req = client
        .get(url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github.raw+json");
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }

    let resp = req.send().await.map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("GitHub API request failed: {e}"),
    })?;

    let status = resp.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(SkillError::AuthRequired {
            git_url: git_url.to_string(),
        });
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(SkillError::Fetch {
            git_url: git_url.to_string(),
            message: format!("GitHub API returned {status}: {body}"),
        });
    }

    let body = resp.text().await.map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("failed to read GitHub response body: {e}"),
    })?;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubFetcher {
        files: Vec<FetchedSkillFile>,
    }

    #[async_trait]
    impl SkillFetcher for StubFetcher {
        async fn fetch_skill_files(
            &self,
            _remote_url: &str,
            _token: Option<&str>,
        ) -> Result<Vec<FetchedSkillFile>, SkillError> {
            Ok(self.files.clone())
        }
    }

    #[tokio::test]
    async fn stub_returns_injected_files() {
        let stub = StubFetcher {
            files: vec![FetchedSkillFile {
                dir_name: "alpha".into(),
                content: "---\nname: alpha\ndescription: A\n---\n".into(),
            }],
        };
        let files = stub
            .fetch_skill_files("https://github.com/x/y.git", None)
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].dir_name, "alpha");
    }

    #[tokio::test]
    async fn non_github_url_is_unsupported() {
        let fetcher = GitHubSkillFetcher::new(Client::new());
        let err = fetcher
            .fetch_skill_files("https://gitlab.com/foo/bar.git", None)
            .await
            .unwrap_err();
        match err {
            SkillError::UnsupportedProvider { git_url } => {
                assert_eq!(git_url, "https://gitlab.com/foo/bar.git");
            }
            other => panic!("expected UnsupportedProvider, got {other:?}"),
        }
    }
}
