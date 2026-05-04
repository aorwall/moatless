//! GitHub REST API client for PR interactions.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::channel::error::ChannelError;

/// Maximum comment length for GitHub's API (64 KB).
const MAX_COMMENT_LENGTH: usize = 65_536;

/// GitHub PR comment from the API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PrComment {
    pub id: u64,
    #[serde(rename = "user")]
    user: Option<PrUser>,
    pub body: Option<String>,
    pub created_at: String,
}

impl PrComment {
    pub fn author(&self) -> &str {
        self.user.as_ref().map_or("unknown", |u| u.login.as_str())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PrUser {
    login: String,
}

/// GitHub PR metadata from the API.
#[derive(Debug, Clone, Deserialize)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub user: Option<PrInfoUser>,
    pub head: PrRef,
    pub base: PrRef,
}

impl PrInfo {
    pub fn author(&self) -> &str {
        self.user.as_ref().map_or("unknown", |u| u.login.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrInfoUser {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
}

/// GitHub REST API client for PR operations.
pub struct GithubPrApi {
    client: Client,
    token: String,
}

impl GithubPrApi {
    pub fn new(token: &str) -> Self {
        Self {
            client: Client::new(),
            token: token.to_string(),
        }
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "moatless-vibe")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// Post a comment on a PR (via the Issues API).
    pub async fn post_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        body: &str,
    ) -> Result<u64, ChannelError> {
        let url =
            format!("https://api.github.com/repos/{owner}/{repo}/issues/{pr_number}/comments");

        // Truncate if needed
        let body = if body.len() > MAX_COMMENT_LENGTH {
            &body[..MAX_COMMENT_LENGTH]
        } else {
            body
        };

        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("GitHub post_comment failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub post_comment error");
            return Err(ChannelError::ProviderApi(format!(
                "GitHub post_comment returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            ChannelError::ProviderApi(format!("GitHub post_comment parse error: {e}"))
        })?;

        let comment_id = data
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        Ok(comment_id)
    }

    /// List recent comments on a PR.
    pub async fn list_comments(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        limit: usize,
    ) -> Result<Vec<PrComment>, ChannelError> {
        let url =
            format!("https://api.github.com/repos/{owner}/{repo}/issues/{pr_number}/comments");

        let resp = self
            .request(reqwest::Method::GET, &url)
            .query(&[
                ("per_page", limit.to_string()),
                ("direction", "desc".to_string()),
            ])
            .send()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("GitHub list_comments failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::ProviderApi(format!(
                "GitHub list_comments returned {status}: {text}"
            )));
        }

        let comments: Vec<PrComment> = resp.json().await.map_err(|e| {
            ChannelError::ProviderApi(format!("GitHub list_comments parse error: {e}"))
        })?;

        Ok(comments)
    }

    /// Get PR metadata (title, body, author, state, branches).
    pub async fn get_pr(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PrInfo, ChannelError> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{pr_number}");

        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("GitHub get_pr failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::ProviderApi(format!(
                "GitHub get_pr returned {status}: {text}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("GitHub get_pr parse error: {e}")))
    }

    /// Update a PR's body.
    pub async fn update_pr_body(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        body: &str,
    ) -> Result<(), ChannelError> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{pr_number}");

        let resp = self
            .request(reqwest::Method::PATCH, &url)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("GitHub update_pr_body failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub update_pr_body error");
            return Err(ChannelError::ProviderApi(format!(
                "GitHub update_pr_body returned {status}: {text}"
            )));
        }

        Ok(())
    }
}
