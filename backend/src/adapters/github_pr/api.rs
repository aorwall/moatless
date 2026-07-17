//! GitHub REST API client for PR interactions.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use tracing::warn;

use crate::adapters::error::AdapterError;

/// Maximum comment length for GitHub's API (64 KB).
pub(crate) const MAX_COMMENT_LENGTH: usize = 65_536;

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
    /// Whether the PR was merged. Distinguishes `merged` from a plain `closed`
    /// PR (GitHub reports both as `state = "closed"`). Absent on older
    /// payloads, hence optional.
    #[serde(default)]
    pub merged: Option<bool>,
    /// Whether the PR is a draft.
    #[serde(default)]
    pub draft: Option<bool>,
    /// Whether GitHub currently considers the PR mergeable. `None` until the
    /// background mergeability job finishes — GitHub computes this lazily and
    /// the first `GET` on the PR is what kicks the computation off, so a fresh
    /// read is often `null`.
    #[serde(default)]
    pub mergeable: Option<bool>,
    /// Coarse merge state: `clean`, `dirty` (has conflicts), `blocked`,
    /// `behind`, `unstable`, `has_hooks`, `draft`, or `unknown` (not yet
    /// computed). Only present on the single-PR endpoint, never on list/webhook
    /// payloads.
    #[serde(default)]
    pub mergeable_state: Option<String>,
    /// PR web URL (`html_url`).
    #[serde(default)]
    pub html_url: Option<String>,
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

/// Default GitHub REST API base URL.
const DEFAULT_API_BASE: &str = "https://api.github.com";

/// Resolve the GitHub API base URL. `GITHUB_API_BASE_URL` overrides the
/// default — used by E2E tests to point the backend at a mock server (and
/// usable for GitHub Enterprise hosts). Empty values are treated as unset.
fn api_base() -> String {
    std::env::var("GITHUB_API_BASE_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

/// GitHub REST API client for PR operations.
pub struct GithubPrApi {
    client: Client,
    token: String,
    base_url: String,
}

impl GithubPrApi {
    pub fn new(token: &str) -> Self {
        Self {
            client: Client::new(),
            token: token.to_string(),
            base_url: api_base(),
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
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/{pr_number}/comments",
            self.base_url
        );

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

    /// Create a pull request. Returns the raw GitHub PR payload so callers
    /// can extract the fields they need (`number`, `html_url`, ...).
    #[allow(clippy::too_many_arguments)] // reason: mirrors the GitHub create-PR parameter set
    pub async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
        draft: bool,
    ) -> Result<serde_json::Value, AdapterError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls", self.base_url);

        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({
                "title": title,
                "body": body,
                "head": head,
                "base": base,
                "draft": draft,
            }))
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub create_pr failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub create_pr error");
            return Err(AdapterError::AdapterApi(format!(
                "GitHub create_pr returned {status}: {text}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub create_pr parse error: {e}")))
    }

    /// Fetch a repository's default branch name.
    pub async fn get_repo_default_branch(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<String, AdapterError> {
        let url = format!("{}/repos/{owner}/{repo}", self.base_url);

        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub get_repo failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "GitHub get_repo returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub get_repo parse error: {e}")))?;
        data.get("default_branch")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                AdapterError::AdapterApi("GitHub get_repo: missing default_branch".into())
            })
    }

    /// Reply to a PR review comment (threaded review reply).
    ///
    /// Calls `POST /repos/{owner}/{repo}/pulls/{pr}/comments/{cid}/replies` and
    /// returns the new review comment id.
    pub async fn reply_to_review_comment(
        &self,
        owner: &str,
        repo: &str,
        pull_number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<u64, AdapterError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies",
            self.base_url
        );

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
            .map_err(|e| {
                AdapterError::AdapterApi(format!("GitHub reply_to_review_comment failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub reply_to_review_comment error");
            return Err(AdapterError::AdapterApi(format!(
                "GitHub reply_to_review_comment returned {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            AdapterError::AdapterApi(format!("GitHub reply_to_review_comment parse error: {e}"))
        })?;

        data.get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                AdapterError::AdapterApi("GitHub reply_to_review_comment: missing id".into())
            })
    }

    /// Get PR metadata (title, body, author, state, branches).
    pub async fn get_pr(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PrInfo, AdapterError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls/{pr_number}", self.base_url);

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

    /// Fetch PR metadata, polling until GitHub has computed the mergeable
    /// state (or a small retry budget is exhausted).
    ///
    /// GitHub computes mergeability asynchronously: a PR read right after a push
    /// returns `mergeable: null` / `mergeable_state: "unknown"`, and the read
    /// itself is what triggers the background job. We retry a few times with a
    /// short backoff so callers get a settled `clean`/`dirty`/… value. If it
    /// never settles within the budget, the last (possibly `unknown`) `PrInfo`
    /// is returned rather than erroring — callers treat `unknown` as
    /// "inconclusive" and simply don't notify.
    pub async fn get_pr_mergeable(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PrInfo, AdapterError> {
        /// Retries after the initial read (which usually returns `unknown`).
        const MERGEABLE_POLL_RETRIES: usize = 3;
        /// Backoff between reads while GitHub computes mergeability.
        const MERGEABLE_POLL_DELAY: Duration = Duration::from_secs(2);

        let mut pr = self.get_pr(owner, repo, pr_number).await?;
        for _ in 0..MERGEABLE_POLL_RETRIES {
            let settled = pr.mergeable.is_some()
                && pr.mergeable_state.as_deref().is_some_and(|s| s != "unknown");
            // A closed/merged PR never gets a mergeable state — stop early.
            if settled || !pr.state.eq_ignore_ascii_case("open") {
                return Ok(pr);
            }
            tokio::time::sleep(MERGEABLE_POLL_DELAY).await;
            pr = self.get_pr(owner, repo, pr_number).await?;
        }
        Ok(pr)
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
            "{}/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions",
            self.base_url
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
            "{}/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions",
            self.base_url
        );
        self.create_reaction(&url, content).await
    }

    async fn create_reaction(&self, url: &str, content: &str) -> Result<u64, AdapterError> {
        let resp = self
            .request(reqwest::Method::POST, url)
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub create_reaction failed: {e}")))?;

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
            .ok_or_else(|| AdapterError::AdapterApi("GitHub create_reaction: missing id".into()))?;
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
            "{}/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions",
            self.base_url
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
            "{}/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions",
            self.base_url
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
            "{}/repos/{owner}/{repo}/issues/comments/{comment_id}/reactions/{reaction_id}",
            self.base_url
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
            "{}/repos/{owner}/{repo}/pulls/comments/{comment_id}/reactions/{reaction_id}",
            self.base_url
        );
        self.delete_reaction(&url).await
    }

    async fn delete_reaction(&self, url: &str) -> Result<(), AdapterError> {
        let resp = self
            .request(reqwest::Method::DELETE, url)
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("GitHub delete_reaction failed: {e}")))?;
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
            .request(reqwest::Method::GET, &format!("{}/user", self.base_url))
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
        let url = format!("{}/repos/{owner}/{repo}/pulls/{pr_number}", self.base_url);

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
