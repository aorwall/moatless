//! GitHub REST API client for PR interactions.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::adapters::error::AdapterError;

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

/// A single reaction returned by the GitHub reactions API.
#[derive(Debug, Clone, Deserialize)]
pub struct ReactionInfo {
    pub id: u64,
    pub content: String,
    #[serde(default)]
    pub user: Option<ReactionUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReactionUser {
    pub login: String,
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
    ) -> Result<u64, AdapterError> {
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
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub post_comment failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub post_comment error");
            return Err(AdapterError::AdapterApi(format!(
                "GitHub post_comment returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            AdapterError::AdapterApi(format!("GitHub post_comment parse error: {e}"))
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
    ) -> Result<Vec<PrComment>, AdapterError> {
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
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub list_comments failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "GitHub list_comments returned {status}: {text}"
            )));
        }

        let comments: Vec<PrComment> = resp.json().await.map_err(|e| {
            AdapterError::AdapterApi(format!("GitHub list_comments parse error: {e}"))
        })?;

        Ok(comments)
    }

    /// Get PR metadata (title, body, author, state, branches).
    pub async fn get_pr(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PrInfo, AdapterError> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{pr_number}");

        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub get_pr failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "GitHub get_pr returned {status}: {text}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub get_pr parse error: {e}")))
    }

    /// Create a reaction on an issue comment (also covers PR conversation
    /// comments, since they share the issues comments API).
    ///
    /// Returns the GitHub reaction id.
    pub async fn create_issue_comment_reaction(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        content: &str,
    ) -> Result<u64, AdapterError> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions"
        );
        self.create_reaction(&url, content).await
    }

    /// Create a reaction on a PR review (line) comment.
    pub async fn create_review_comment_reaction(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        content: &str,
    ) -> Result<u64, AdapterError> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions"
        );
        self.create_reaction(&url, content).await
    }

    async fn create_reaction(&self, url: &str, content: &str) -> Result<u64, AdapterError> {
        let resp = self
            .request(reqwest::Method::POST, url)
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .map_err(|e| {
                AdapterError::AdapterApi(format!("GitHub create_reaction failed: {e}"))
            })?;

        let status = resp.status();
        // GitHub returns 200 (existing) or 201 (created); both contain the reaction body.
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub create_reaction error");
            return Err(AdapterError::AdapterApi(format!(
                "GitHub create_reaction returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            AdapterError::AdapterApi(format!("GitHub create_reaction parse error: {e}"))
        })?;
        let reaction_id = data
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                AdapterError::AdapterApi("GitHub create_reaction: missing id".into())
            })?;
        Ok(reaction_id)
    }

    /// List reactions on an issue comment.
    pub async fn list_issue_comment_reactions(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
    ) -> Result<Vec<ReactionInfo>, AdapterError> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions"
        );
        self.list_reactions(&url).await
    }

    /// List reactions on a PR review comment.
    pub async fn list_review_comment_reactions(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
    ) -> Result<Vec<ReactionInfo>, AdapterError> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions"
        );
        self.list_reactions(&url).await
    }

    async fn list_reactions(&self, url: &str) -> Result<Vec<ReactionInfo>, AdapterError> {
        let resp = self
            .request(reqwest::Method::GET, url)
            .query(&[("per_page", "100")])
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub list_reactions failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "GitHub list_reactions returned {status}: {text}"
            )));
        }

        resp.json().await.map_err(|e| {
            AdapterError::AdapterApi(format!("GitHub list_reactions parse error: {e}"))
        })
    }

    /// Delete a reaction from an issue comment by reaction id.
    pub async fn delete_issue_comment_reaction(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        reaction_id: u64,
    ) -> Result<(), AdapterError> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions/{reaction_id}"
        );
        self.delete_reaction(&url).await
    }

    /// Delete a reaction from a PR review comment by reaction id.
    pub async fn delete_review_comment_reaction(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        reaction_id: u64,
    ) -> Result<(), AdapterError> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions/{reaction_id}"
        );
        self.delete_reaction(&url).await
    }

    async fn delete_reaction(&self, url: &str) -> Result<(), AdapterError> {
        let resp = self
            .request(reqwest::Method::DELETE, url)
            .send()
            .await
            .map_err(|e| {
                AdapterError::AdapterApi(format!("GitHub delete_reaction failed: {e}"))
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "GitHub delete_reaction returned {status}: {text}"
            )));
        }
        Ok(())
    }

    /// Look up the authenticated bot's GitHub login. Used to filter reactions
    /// when removing.
    pub async fn authenticated_login(&self) -> Result<String, AdapterError> {
        let resp = self
            .request(reqwest::Method::GET, "https://api.github.com/user")
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub user failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "GitHub user returned {status}: {text}"
            )));
        }
        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub user parse error: {e}")))?;
        data.get("login")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AdapterError::AdapterApi("GitHub user: missing login".into()))
    }

    /// Update a PR's body.
    pub async fn update_pr_body(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        body: &str,
    ) -> Result<(), AdapterError> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{pr_number}");

        let resp = self
            .request(reqwest::Method::PATCH, &url)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub update_pr_body failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub update_pr_body error");
            return Err(AdapterError::AdapterApi(format!(
                "GitHub update_pr_body returned {status}: {text}"
            )));
        }

        Ok(())
    }
}
