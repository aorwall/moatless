//! Typed provider HTTP extraction, response adaptation, and OpenAPI metadata.

use std::sync::Arc;
use axum::Json;
use axum::extract::{Path, State};
use moatless_core::http_auth::AuthUser;
use moatless_adapter_model::error::AdapterError;
use moatless_ports::task::TaskReader;
use moatless_api_schemas::github::{
    GithubBindPrRequest, GithubBindPrResponse, GithubCommentRequest, GithubCommentResponse,
    GithubCreatePrRequest, GithubCreatePrResponse, GithubExternalEventRequest,
    GithubExternalEventResponse, GithubReplyRequest,
};
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_ports::task::TranscriptAppend;
use super::adapter::GithubPrAdapter;

#[utoipa::path(
    post,
    path = "/api/v1/github/repos/{owner}/{repo}/pulls",
    tag = "github",
    params(
        ("owner" = String, Path, description = "Repository owner (org or user)"),
        ("repo" = String, Path, description = "Repository name"),
    ),
    request_body = GithubCreatePrRequest,
    responses(
        (status = 200, description = "Pull request created", body = GithubCreatePrResponse),
        (status = 400, description = "Bad request / adapter not configured", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 502, description = "GitHub API error / no usable GitHub token", body = moatless_api_schemas::error::ErrorBody),
    )
)]
#[expect(clippy::too_many_arguments, reason = "the authorized mutation receives separate provider, binding, transcript, task, and caller dependencies")]
pub async fn create_pr(
    State(connector): State<Option<Arc<moatless_loops::loops::firing::listen::Connector>>>,
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    State(recently_sent): State<Arc<RecentlySent>>,
    State(messages): State<Arc<dyn TranscriptAppend>>,
    auth: AuthUser,
    Path((owner, repo)): Path<(String, String)>,
    Json(req): Json<GithubCreatePrRequest>,
) -> Result<Json<GithubCreatePrResponse>, AdapterError> {
    super::mutations::create_pr(
        connector,
        github_pr_provider,
        tasks,
        recently_sent,
        messages,
        auth.scope_context(),
        (owner, repo),
        req,
    )
    .await
    .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/v1/github/repos/{owner}/{repo}/issues/{number}/comments",
    tag = "github",
    params(
        ("owner" = String, Path, description = "Repository owner (org or user)"),
        ("repo" = String, Path, description = "Repository name"),
        ("number" = u64, Path, description = "PR or issue number"),
    ),
    request_body = GithubCommentRequest,
    responses(
        (status = 200, description = "Comment posted", body = GithubCommentResponse),
        (status = 400, description = "Bad request / adapter not configured", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 502, description = "GitHub API error / no usable GitHub token", body = moatless_api_schemas::error::ErrorBody),
    )
)]
#[expect(clippy::too_many_arguments, reason = "the authorized mutation receives separate provider, binding, transcript, task, and caller dependencies")]
pub async fn post_comment(
    State(connector): State<Option<Arc<moatless_loops::loops::firing::listen::Connector>>>,
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    State(recently_sent): State<Arc<RecentlySent>>,
    State(messages): State<Arc<dyn TranscriptAppend>>,
    auth: AuthUser,
    Path((owner, repo, number)): Path<(String, String, u64)>,
    Json(req): Json<GithubCommentRequest>,
) -> Result<Json<GithubCommentResponse>, AdapterError> {
    super::mutations::post_comment(
        connector,
        github_pr_provider,
        tasks,
        recently_sent,
        messages,
        auth.scope_context(),
        (owner, repo, number),
        req,
    )
    .await
    .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/v1/github/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies",
    tag = "github",
    params(
        ("owner" = String, Path, description = "Repository owner (org or user)"),
        ("repo" = String, Path, description = "Repository name"),
        ("number" = u64, Path, description = "PR number"),
        ("comment_id" = u64, Path, description = "Review comment id to reply to"),
    ),
    request_body = GithubReplyRequest,
    responses(
        (status = 200, description = "Threaded reply posted", body = GithubCommentResponse),
        (status = 400, description = "Bad request / adapter not configured", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 502, description = "GitHub API error / no usable GitHub token", body = moatless_api_schemas::error::ErrorBody),
    )
)]
#[expect(clippy::too_many_arguments, reason = "the authorized mutation receives separate provider, binding, transcript, task, and caller dependencies")]
pub async fn reply_to_review_comment(
    State(connector): State<Option<Arc<moatless_loops::loops::firing::listen::Connector>>>,
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    State(recently_sent): State<Arc<RecentlySent>>,
    State(messages): State<Arc<dyn TranscriptAppend>>,
    auth: AuthUser,
    Path((owner, repo, number, comment_id)): Path<(String, String, u64, u64)>,
    Json(req): Json<GithubReplyRequest>,
) -> Result<Json<GithubCommentResponse>, AdapterError> {
    super::mutations::reply_to_review_comment(
        connector,
        github_pr_provider,
        tasks,
        recently_sent,
        messages,
        auth.scope_context(),
        (owner, repo, number, comment_id),
        req,
    )
    .await
    .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/v1/github/repos/{owner}/{repo}/pulls/{number}/bind",
    tag = "github",
    params(
        ("owner" = String, Path, description = "Repository owner (org or user)"),
        ("repo" = String, Path, description = "Repository name"),
        ("number" = u64, Path, description = "PR number"),
    ),
    request_body = GithubBindPrRequest,
    responses(
        (status = 200, description = "Pull request bound to task", body = GithubBindPrResponse),
        (status = 400, description = "Bad request / adapter not configured", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn bind_pr(
    State(connector): State<Option<Arc<moatless_loops::loops::firing::listen::Connector>>>,
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    auth: AuthUser,
    Path((owner, repo, number)): Path<(String, String, u64)>,
    Json(req): Json<GithubBindPrRequest>,
) -> Result<Json<GithubBindPrResponse>, AdapterError> {
    super::mutations::bind_pr(
        connector,
        github_pr_provider,
        tasks,
        auth.scope_context(),
        (owner, repo, number),
        req,
    )
    .await
    .map(Json)
}

pub use super::pull_request_lookup::{
    find_open_pull_request_for_head, resolve_reference_for_dialog, DialogPullRequest,
};

#[utoipa::path(
    post,
    path = "/api/v1/github/external-events",
    tag = "github",
    request_body = GithubExternalEventRequest,
    responses(
        (status = 200, description = "External event recorded", body = GithubExternalEventResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
    )
)]
/// Record a GitHub event the caller produced outside the typed routes above.
///
/// `moat gh api` reaches GitHub directly, because the endpoint surface of `gh
/// api` is too wide to type, so the side effect the typed routes perform inline
/// has to be requested separately. The command that made the change is the one
/// that calls this, with the response in hand.
pub async fn record_external_event(
    State(tasks): State<Arc<dyn TaskReader>>,
    State(recently_sent): State<Arc<RecentlySent>>,
    State(messages): State<Arc<dyn TranscriptAppend>>,
    auth: AuthUser,
    Json(req): Json<GithubExternalEventRequest>,
) -> Result<Json<GithubExternalEventResponse>, AdapterError> {
    super::mutations::record_external_event(tasks, recently_sent, messages, auth.scope_context(), req).await.map(Json)
}
