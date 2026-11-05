//! Reaction calls on issue and review comments.

use tracing::warn;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

use super::api::{GithubPrApi, ReactionInfo};
use crate::rejection::rejection;

impl GithubPrApi {
    /// Create a reaction on an issue comment (also covers PR conversation
    /// comments, since they share the issues comments API).
    ///
    /// Returns the GitHub reaction id.
    pub(super) async fn create_issue_comment_reaction(
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
    pub(super) async fn create_review_comment_reaction(
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
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub create_reaction", e))?;

        let status = resp.status();
        // GitHub returns 200 (existing) or 201 (created); both contain the reaction body.
        if !status.is_success() {
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "GitHub create_reaction error");
            return Err(rejection("GitHub create_reaction", status, &headers, &text));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub create_reaction", e)
        })?;
        let reaction_id = data
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("GitHub create_reaction: missing id".into())
            })?;
        Ok(reaction_id)
    }

    /// List reactions on an issue comment.
    pub(super) async fn list_issue_comment_reactions(
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
    pub(super) async fn list_review_comment_reactions(
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
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub list_reactions", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub list_reactions", status, &headers, &text));
        }

        resp.json().await.map_err(|e| {
            SharedAdapterError::provider_response_invalid("GitHub list_reactions", e).into()
        })
    }

    /// Delete a reaction from an issue comment by reaction id.
    pub(super) async fn delete_issue_comment_reaction(
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
    pub(super) async fn delete_review_comment_reaction(
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
            .map_err(|e| SharedAdapterError::provider_unreachable("GitHub delete_reaction", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(rejection("GitHub delete_reaction", status, &headers, &text));
        }
        Ok(())
    }
}
