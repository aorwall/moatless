//! GitHub API schemas — request/response DTOs for the typed
//! `/api/v1/github/*` endpoints. These let sandbox agents create PRs,
//! post comments, and bind existing PRs through the backend (via `moat gh`),
//! so task bindings, the `Moatless task:` footer, and webhook echo dedup happen
//! as part of the API call instead of being reconstructed from shell-command
//! parsing.

use serde::{Deserialize, Serialize};

/// `POST /api/v1/github/repos/{owner}/{repo}/pulls` — create a pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GithubCreatePrRequest {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Head branch the PR is created from. The CLI infers this from the
    /// current git branch when the agent omits `--head`.
    pub head: String,
    /// Base branch. Defaults to the repository's default branch when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
    /// Task to bind the created PR to. When provided, a `task_bindings` row
    /// is upserted so inbound PR comments route back to this task, and a
    /// `Moatless task: <url>` footer is appended to the PR body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// Response for PR creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GithubCreatePrResponse {
    pub number: u64,
    pub html_url: String,
    pub title: String,
    pub state: String,
    /// Set only when the request included a `task_id` and the binding upsert
    /// succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
}

/// `POST /api/v1/github/repos/{owner}/{repo}/issues/{number}/comments` —
/// post a comment on a PR or issue (GitHub's issues API covers both).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GithubCommentRequest {
    pub body: String,
    /// Task to bind the PR/issue to and dedup the comment's webhook echo
    /// against. See [`GithubCreatePrRequest::task_id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// `POST /api/v1/github/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies`
/// — threaded reply to a PR review (inline) comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GithubReplyRequest {
    pub body: String,
    /// Task to bind the PR to and dedup the reply's webhook echo against.
    /// See [`GithubCreatePrRequest::task_id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// Response for comment creation (top-level comments and threaded
/// review-comment replies alike).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GithubCommentResponse {
    pub id: u64,
    pub html_url: String,
    /// Set only when the request included a `task_id` and the binding upsert
    /// succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
}

/// `POST /api/v1/github/repos/{owner}/{repo}/pulls/{number}/bind` — bind an
/// existing PR to a task without mutating GitHub.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GithubBindPrRequest {
    pub task_id: String,
}

/// Response for binding an existing PR.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GithubBindPrResponse {
    pub number: u64,
    pub html_url: String,
    /// Set when the task binding upsert succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
}
