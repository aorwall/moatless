//! Linear GraphQL API client.

use reqwest::Client;
use serde::Deserialize;
use tracing::warn;

use crate::adapters::error::AdapterError;

const LINEAR_GRAPHQL_URL: &str = "https://api.linear.app/graphql";

/// Linear comment-on-issue.
#[derive(Debug, Clone, Deserialize)]
pub struct LinearComment {
    pub id: String,
    pub body: String,
    #[serde(default)]
    pub user: Option<LinearUser>,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
}

impl LinearComment {
    pub fn author(&self) -> &str {
        self.user.as_ref().map_or("unknown", |u| u.name.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinearUser {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// Issue metadata (used by `fetch_context` to backfill title/description).
#[derive(Debug, Clone, Deserialize)]
pub struct IssueData {
    pub id: String,
    pub identifier: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// Authenticated bot identity (used at registration time for log lines).
#[derive(Debug, Clone, Deserialize)]
pub struct ViewerInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// Linear GraphQL API client.
pub struct LinearApi {
    client: Client,
    token: String,
}

impl LinearApi {
    pub fn new(token: &str) -> Self {
        Self {
            client: Client::new(),
            token: token.to_string(),
        }
    }

    /// Send a GraphQL request and surface errors via `AdapterError::AdapterApi`.
    async fn graphql(&self, body: serde_json::Value) -> Result<serde_json::Value, AdapterError> {
        let resp = self
            .client
            .post(LINEAR_GRAPHQL_URL)
            // Linear personal API keys are sent as a raw token in the
            // Authorization header — OAuth tokens use `Bearer`. Callers pass
            // whichever they hold; we forward verbatim.
            .header("Authorization", &self.token)
            .header("Content-Type", "application/json")
            .header("User-Agent", "moatless-vibe")
            .json(&body)
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("Linear request failed: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("Linear response read failed: {e}")))?;

        if !status.is_success() {
            warn!(status = %status, body = %text, "Linear GraphQL HTTP error");
            return Err(AdapterError::AdapterApi(format!(
                "Linear API returned {status}: {text}"
            )));
        }

        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AdapterError::AdapterApi(format!("Linear JSON parse error: {e}")))?;

        if let Some(errors) = value.get("errors").and_then(serde_json::Value::as_array) {
            if !errors.is_empty() {
                warn!(errors = %serde_json::Value::Array(errors.clone()), "Linear GraphQL returned errors");
                return Err(AdapterError::AdapterApi(format!(
                    "Linear GraphQL errors: {errors:?}"
                )));
            }
        }

        Ok(value)
    }

    /// Create a comment on an issue.
    pub async fn comment_create(&self, issue_id: &str, body: &str) -> Result<String, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($input: CommentCreateInput!) { \
                    commentCreate(input: $input) { success comment { id } } \
                }",
                "variables": {
                    "input": {
                        "issueId": issue_id,
                        "body": body,
                    }
                }
            }))
            .await?;

        let success = value
            .pointer("/data/commentCreate/success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if !success {
            return Err(AdapterError::AdapterApi(
                "Linear commentCreate returned success=false".to_string(),
            ));
        }

        let id = value
            .pointer("/data/commentCreate/comment/id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::AdapterApi("Linear commentCreate missing comment.id".to_string())
            })?;

        Ok(id.to_string())
    }

    /// Get the authenticated viewer (used at registration time to log identity).
    pub async fn viewer(&self) -> Result<ViewerInfo, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "query { viewer { id name email } }"
            }))
            .await?;

        let viewer = value
            .pointer("/data/viewer")
            .ok_or_else(|| AdapterError::AdapterApi("Linear viewer missing".into()))?;

        serde_json::from_value(viewer.clone())
            .map_err(|e| AdapterError::AdapterApi(format!("Linear viewer parse error: {e}")))
    }

    /// Fetch issue metadata by stable UUID.
    pub async fn issue(&self, id: &str) -> Result<IssueData, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "query($id: String!) { \
                    issue(id: $id) { id identifier title description url } \
                }",
                "variables": { "id": id }
            }))
            .await?;

        let issue = value
            .pointer("/data/issue")
            .ok_or_else(|| AdapterError::AdapterApi(format!("Linear issue {id} not found")))?;

        serde_json::from_value(issue.clone())
            .map_err(|e| AdapterError::AdapterApi(format!("Linear issue parse error: {e}")))
    }

    /// List recent comments on an issue (newest first, capped by `limit`).
    pub async fn list_comments(
        &self,
        issue_id: &str,
        limit: usize,
    ) -> Result<Vec<LinearComment>, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "query($id: String!, $first: Int!) { \
                    issue(id: $id) { \
                        comments(first: $first, orderBy: createdAt) { \
                            nodes { id body createdAt user { id name email } } \
                        } \
                    } \
                }",
                "variables": { "id": issue_id, "first": limit as i64 }
            }))
            .await?;

        let nodes = value
            .pointer("/data/issue/comments/nodes")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(vec![]));

        serde_json::from_value(nodes)
            .map_err(|e| AdapterError::AdapterApi(format!("Linear list_comments parse error: {e}")))
    }
}
