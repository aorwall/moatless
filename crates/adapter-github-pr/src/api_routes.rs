//! Typed `/api/v1/github/*` API surface for sandbox agents (via `moat gh`).
//!
//! Mirrors the `moat linear` pattern: rather than leaving the agent to shell
//! out to `gh pr create` / `gh pr comment` and having the side effects
//! reconstructed afterwards, these endpoints make them part of the API call
//! itself:
//! - task↔PR binding upsert (`connect_github_binding`)
//! - `Moatless task: <url>` footer baked into the PR body at creation
//! - webhook echo dedup (`recently_sent`) for the PR-opened / comment events
//!
//! Tokens come from the credential the task's Workspace pins, else the calling
//! user's own GitHub row, and nothing else — the same rule the sandbox's
//! `GITHUB_TOKEN` follows, so the identity that pushed the branch is the
//! identity that opens the PR. That is also what lets these routes reach a
//! repository the task isn't bound to: the access held by the account those two
//! rules name is the only thing that decides.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use moatless_api_schemas::github::{
    GithubBindPrRequest, GithubBindPrResponse, GithubCommentRequest, GithubCommentResponse,
    GithubCreatePrRequest, GithubExternalEventRequest, GithubExternalEventResponse,
    GithubExternalEventSubject,
    GithubCreatePrResponse, GithubReplyRequest,
};
use serde_json::{Value, json};
use tracing::{info, warn};

use moatless_backend_core::task::{ExternalEvent, TaskReader, TranscriptAppend};
use moatless_loops::adapters::common::{ensure_task_access_only, ensure_task_access_with};
use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::footer::FOOTER_MARKER;
use moatless_loops::adapters::AdapterDestinationBuilder;
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_types::access_control::Actor;
use moatless_identity::extractors::AuthUser;
use moatless_identity::git_host_repo::GitHostRepo;

use super::adapter::GithubPrAdapter;
use super::api::{GithubPrApi, MAX_COMMENT_LENGTH};
use super::external_id;

/// Extract the configured GitHub PR adapter or answer "adapter not configured".
///
/// The typed API resolves credentials, the frontend-URL footer, and the binding
/// Connector through this one provider handle rather than an `AppState`, mirroring
/// [`super::handlers::github_webhook`].
/// Provider label recorded on an external event, and the `external_adapter` an inbound
/// event is matched under.
const ADAPTER_LABEL: &str = "github_pr";

fn require_provider(
    provider: Option<Arc<GithubPrAdapter>>,
) -> Result<Arc<GithubPrAdapter>, AdapterError> {
    provider.ok_or_else(|| SharedAdapterError::AdapterNotConfigured("github_pr".into()).into())
}

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
pub async fn create_pr(
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    State(recently_sent): State<Arc<RecentlySent>>,
    State(messages): State<Arc<dyn TranscriptAppend>>,
    auth: AuthUser,
    Path((owner, repo)): Path<(String, String)>,
    Json(req): Json<GithubCreatePrRequest>,
) -> Result<Json<GithubCreatePrResponse>, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let ctx = auth.scope_context();
    let workspace_id = task_workspace(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;

    let api = resolve_github_api(
        &adapter,
        &auth.user_id.to_string(),
        workspace_id.as_deref(),
        &owner,
        &repo,
    )
    .await?;

    let base = match req.base.as_deref() {
        Some(b) => b.to_string(),
        None => api.get_repo_default_branch(&owner, &repo).await?,
    };

    let body = build_pr_body(
        req.body.as_deref().unwrap_or(""),
        task_footer_url(adapter.frontend_url(), req.task_id.as_deref()).as_deref(),
    );

    let created = match api
        .create_pr(
            &owner,
            &repo,
            &req.title,
            &body,
            &req.head,
            &base,
            req.draft.unwrap_or(false),
        )
        .await
    {
        Ok(pr) => CreatedPr::from_created(&pr, &req.title)?,
        // A head branch that already has an open pull request is the one
        // non-success that is not a failure: GitHub answers 422, and we bind the
        // pull request that already exists instead of surfacing the 422. Any
        // other create failure — and a 422 with no open pull request to find —
        // passes through unchanged.
        Err(create_error) => match api.find_open_pr_for_head(&owner, &repo, &req.head).await {
            Ok(Some(existing)) => CreatedPr::from_existing(&existing, &owner, &repo),
            _ => return Err(create_error),
        },
    };

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        let bid = bind_task_to_pr(
            &adapter,
            &owner,
            &repo,
            created.number,
            task_id,
            &auth.user_id.to_string(),
        )
        .await;
        note_external_event(
            recently_sent.as_ref(),
            messages.as_ref(),
            task_id,
            &external_id::pr_opened(created.number),
            &owner,
            &repo,
        )
        .await;
        bid
    } else {
        None
    };

    Ok(Json(GithubCreatePrResponse {
        number: created.number,
        html_url: created.html_url,
        title: created.title,
        state: created.state,
        binding_id,
        opened_existing: created.opened_existing,
    }))
}

/// The pull request the create endpoint answers with, whether GitHub created it
/// or the head branch already had one open.
struct CreatedPr {
    number: u64,
    html_url: String,
    title: String,
    state: String,
    opened_existing: bool,
}

impl CreatedPr {
    /// From the raw payload GitHub returns on a successful create.
    fn from_created(pr: &Value, requested_title: &str) -> Result<Self, AdapterError> {
        Ok(Self {
            number: pr.get("number").and_then(Value::as_u64).ok_or_else(|| {
                SharedAdapterError::AdapterApi("GitHub create_pr: missing number".into())
            })?,
            html_url: pr
                .get("html_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            title: pr
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(requested_title)
                .to_string(),
            state: pr
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("open")
                .to_string(),
            opened_existing: false,
        })
    }

    /// From the open pull request the head branch already had.
    fn from_existing(pr: &super::api::PrInfo, owner: &str, repo: &str) -> Self {
        Self {
            number: pr.number,
            html_url: pr
                .html_url
                .clone()
                .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/pull/{}", pr.number)),
            title: pr.title.clone(),
            state: pr.state.clone(),
            opened_existing: true,
        }
    }
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
pub async fn post_comment(
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    State(recently_sent): State<Arc<RecentlySent>>,
    State(messages): State<Arc<dyn TranscriptAppend>>,
    auth: AuthUser,
    Path((owner, repo, number)): Path<(String, String, u64)>,
    Json(req): Json<GithubCommentRequest>,
) -> Result<Json<GithubCommentResponse>, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let ctx = auth.scope_context();
    let workspace_id = task_workspace(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;

    let api = resolve_github_api(
        &adapter,
        &auth.user_id.to_string(),
        workspace_id.as_deref(),
        &owner,
        &repo,
    )
    .await?;

    let body = body_with_task_footer(
        tasks.as_ref(),
        adapter.frontend_url(),
        req.task_id.as_deref(),
        &req.body,
    )
    .await;
    let comment_id = api.post_comment(&owner, &repo, number, &body).await?;

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        // Note the echo dedup BEFORE the (slower) binding upsert so the
        // inbound webhook can't race past it.
        note_external_event(
            recently_sent.as_ref(),
            messages.as_ref(),
            task_id,
            &external_id::issue_comment(comment_id),
            &owner,
            &repo,
        )
        .await;
        bind_task_to_pr(
            &adapter,
            &owner,
            &repo,
            number,
            task_id,
            &auth.user_id.to_string(),
        )
        .await
    } else {
        None
    };

    // GitHub redirects `/issues/<n>#issuecomment-<id>` to `/pull/<n>` for PRs,
    // so this URL form works for both PRs and issues without an extra fetch.
    let html_url =
        format!("https://github.com/{owner}/{repo}/issues/{number}#issuecomment-{comment_id}");

    Ok(Json(GithubCommentResponse {
        id: comment_id,
        html_url,
        binding_id,
    }))
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
pub async fn reply_to_review_comment(
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    State(recently_sent): State<Arc<RecentlySent>>,
    State(messages): State<Arc<dyn TranscriptAppend>>,
    auth: AuthUser,
    Path((owner, repo, number, comment_id)): Path<(String, String, u64, u64)>,
    Json(req): Json<GithubReplyRequest>,
) -> Result<Json<GithubCommentResponse>, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let ctx = auth.scope_context();
    let workspace_id = task_workspace(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;

    let api = resolve_github_api(
        &adapter,
        &auth.user_id.to_string(),
        workspace_id.as_deref(),
        &owner,
        &repo,
    )
    .await?;

    let body = body_with_task_footer(
        tasks.as_ref(),
        adapter.frontend_url(),
        req.task_id.as_deref(),
        &req.body,
    )
    .await;
    let new_id = api
        .reply_to_review_comment(&owner, &repo, number, comment_id, &body)
        .await?;

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        // Note the echo dedup BEFORE the (slower) binding upsert so the
        // inbound webhook can't race past it.
        note_external_event(
            recently_sent.as_ref(),
            messages.as_ref(),
            task_id,
            &external_id::review_comment(new_id),
            &owner,
            &repo,
        )
        .await;
        bind_task_to_pr(
            &adapter,
            &owner,
            &repo,
            number,
            task_id,
            &auth.user_id.to_string(),
        )
        .await
    } else {
        None
    };

    let html_url = format!("https://github.com/{owner}/{repo}/pull/{number}#discussion_r{new_id}");

    Ok(Json(GithubCommentResponse {
        id: new_id,
        html_url,
        binding_id,
    }))
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
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    auth: AuthUser,
    Path((owner, repo, number)): Path<(String, String, u64)>,
    Json(req): Json<GithubBindPrRequest>,
) -> Result<Json<GithubBindPrResponse>, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let ctx = auth.scope_context();
    let workspace_id = task_workspace(tasks.as_ref(), &ctx, Some(&req.task_id)).await?;

    let api = resolve_github_api(
        &adapter,
        &auth.user_id.to_string(),
        workspace_id.as_deref(),
        &owner,
        &repo,
    )
    .await?;
    let pr = api.get_pr(&owner, &repo, number).await?;
    let html_url = pr
        .html_url
        .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/pull/{number}"));

    let binding_id = bind_task_to_pr(
        &adapter,
        &owner,
        &repo,
        number,
        &req.task_id,
        &auth.user_id.to_string(),
    )
    .await;

    Ok(Json(GithubBindPrResponse {
        number,
        html_url,
        binding_id,
    }))
}

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
    let ctx = auth.scope_context();
    ensure_task_access_only(tasks.as_ref(), &ctx, &req.task_id).await?;

    let external_id = match &req.subject {
        GithubExternalEventSubject::ReviewComment(id) => external_id::review_comment(*id),
        GithubExternalEventSubject::Review(id) => external_id::review(*id),
        GithubExternalEventSubject::IssueComment(id) => external_id::issue_comment(*id),
    };
    recently_sent.note(&req.task_id, &external_id);
    let recorded = record_durable_event(
        messages.as_ref(),
        &req.task_id,
        &external_id,
        req.source_name.as_deref(),
    )
    .await;

    info!(
        task_id = %req.task_id,
        subject = ?req.subject,
        recorded,
        "Recorded a GitHub event the task made directly"
    );
    Ok(Json(GithubExternalEventResponse { recorded }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Append the `Moatless task: <url>` footer to a body.
///
/// Baked into the PR body at creation so the link is there from the first
/// revision, and so the inbound echo filter can recover the authoring task from
/// any comment quoting it.
fn compose_footer(existing: &str, task_url: &str) -> String {
    let trimmed = existing.trim_end_matches('\n').to_string();
    if trimmed.is_empty() {
        format!("{FOOTER_MARKER} {task_url}\n")
    } else {
        let mut out = trimmed;
        let _ = write!(out, "\n\n---\n{FOOTER_MARKER} {task_url}\n");
        out
    }
}

/// Record `external_id` as `task_id`'s own work, in memory and durably.
///
/// The in-memory note closes the race where the webhook lands before anything
/// has committed; the durable row is what still answers after a restart, and on
/// the replica that did not serve this request.
async fn note_external_event(
    recently_sent: &RecentlySent,
    messages: &dyn TranscriptAppend,
    task_id: &str,
    external_id: &str,
    owner: &str,
    repo: &str,
) {
    recently_sent.note(task_id, external_id);
    record_durable_event(
        messages,
        task_id,
        external_id,
        Some(&format!("{owner}/{repo}")),
    )
    .await;
}

/// The durable half, and whether it landed.
///
/// Best effort: a failure costs one echoed event on a replica that did not
/// serve this request, and the in-memory note has already landed here.
async fn record_durable_event(
    messages: &dyn TranscriptAppend,
    task_id: &str,
    external_id: &str,
    source_name: Option<&str>,
) -> bool {
    match messages
        .record_external_event(
            task_id,
            &ExternalEvent {
                external_id: external_id.to_string(),
                provider: ADAPTER_LABEL.to_string(),
                source_name: source_name.map(str::to_string),
            },
        )
        .await
    {
        Ok(()) => true,
        Err(e) => {
            warn!(task_id, external_id, error = %e, "Failed to record a GitHub event the task made");
            false
        }
    }
}

/// Authorize the caller against the Task and answer the Workspace it runs in.
///
/// The same `NotFound`-on-denied rule the routes had before they needed the
/// Workspace; a route naming no Task authorizes nothing and has no Workspace to
/// report.
async fn task_workspace(
    tasks: &dyn TaskReader,
    ctx: &moatless_types::access_control::ScopeContext,
    task_id: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(task_id) = task_id else {
        return Ok(None);
    };
    Ok(ensure_task_access_with(tasks, ctx, task_id)
        .await?
        .workspace_id)
}

/// Resolve a GitHub API client for `user_id` acting on `workspace_id`, minted
/// against `owner/repo_name`.
///
/// `user_id` is the task's run-as identity — the sandbox's `X-User-Token` JWT
/// and its `GITHUB_TOKEN` are cut from the same `grant.ctx.user_id`. Naming the
/// task's Workspace is what makes this route present the same account the
/// sandbox does: a Workspace that pins a GitHub credential pins it for both.
/// The credential rule (the Workspace's pin or the identity's own row, no scope
/// walk, no repository or global fallback, fails closed) lives on
/// `GithubPrAuth::client_for`; this wrapper just names the provider handle the
/// route reaches it through.
async fn resolve_github_api(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo_name: &str,
) -> Result<GithubPrApi, AdapterError> {
    adapter
        .auth()
        .client_for(
            &Actor::of(user_id).on_workspace(workspace_id),
            &GitHostRepo::github(owner, repo_name),
        )
        .await
}

/// A pull request resolved for the T3 attach-PR dialog and the creation-time
/// binding, in the shape T3's `GitResolvedPullRequest` reports.
pub struct DialogPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub base_ref: String,
    pub head_ref: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
}

/// Resolve an attach-PR reference against the cwd's `repo_owner/repo_name`,
/// following an `owner/repo#N` or PR-URL reference to the repository it names.
/// `Ok(None)` when the reference is not one this resolver understands.
pub async fn resolve_reference_for_dialog(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    repo_owner: &str,
    repo_name: &str,
    reference: &str,
) -> Result<Option<DialogPullRequest>, AdapterError> {
    let Some((owner, repo, number)) =
        parse_pull_request_reference(reference, repo_owner, repo_name)
    else {
        return Ok(None);
    };
    let api = resolve_github_api(adapter, user_id, workspace_id, &owner, &repo).await?;
    let pr = api.get_pr(&owner, &repo, number).await?;
    Ok(Some(to_dialog_pull_request(&pr, &owner, &repo)))
}

/// The open pull request whose head branch is `head`, for binding a Task created
/// on a PR's head branch.
pub async fn find_open_pull_request_for_head(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    head: &str,
) -> Result<Option<DialogPullRequest>, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    Ok(api
        .find_open_pr_for_head(owner, repo, head)
        .await?
        .map(|pr| to_dialog_pull_request(&pr, owner, repo)))
}

/// Parse `N`, `owner/repo#N`, or a `github.com/owner/repo/pull/N` URL into the
/// repository and number to resolve. A bare number names the cwd's repository.
fn parse_pull_request_reference(
    reference: &str,
    default_owner: &str,
    default_repo: &str,
) -> Option<(String, String, u64)> {
    let reference = reference.trim();
    if let Ok(number) = reference.parse::<u64>() {
        return Some((default_owner.to_string(), default_repo.to_string(), number));
    }
    if let Some((repo_part, number_part)) = reference.split_once('#') {
        let (owner, repo) = repo_part.trim().split_once('/')?;
        let number = number_part.trim().parse().ok()?;
        let (owner, repo) = (owner.trim(), repo.trim());
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some((owner.to_string(), repo.to_string(), number));
    }
    // A full pull-request URL: `.../{owner}/{repo}/pull/{number}`.
    let path = reference.split("github.com/").nth(1)?.trim_end_matches('/');
    let mut segments = path.split('/');
    let owner = segments.next()?.trim();
    let repo = segments.next()?.trim();
    if segments.next()? != "pull" {
        return None;
    }
    let number = segments.next()?.trim().parse().ok()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string(), number))
}

fn to_dialog_pull_request(pr: &super::api::PrInfo, owner: &str, repo: &str) -> DialogPullRequest {
    let state = if pr.merged == Some(true) {
        "merged"
    } else if pr.state.as_str() == "closed" {
        "closed"
    } else {
        "open"
    };
    let url = pr
        .html_url
        .clone()
        .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/pull/{}", pr.number));
    let title = if pr.title.trim().is_empty() {
        format!("#{}", pr.number)
    } else {
        pr.title.clone()
    };
    DialogPullRequest {
        number: pr.number,
        title,
        url,
        base_ref: pr.base.ref_name.clone(),
        head_ref: pr.head.ref_name.clone(),
        state: state.to_string(),
    }
}

/// Build the task URL for the PR-body footer. Returns `None` when no task is
/// bound or the frontend URL isn't configured.
fn task_footer_url(frontend_url: &str, task_id: Option<&str>) -> Option<String> {
    let task_id = task_id?;
    let frontend_url = frontend_url.trim_end_matches('/');
    if frontend_url.is_empty() {
        return None;
    }
    Some(format!("{frontend_url}/tasks/{task_id}"))
}

/// Compose the PR body, truncated to GitHub's limit, with the
/// `Moatless task:` footer baked in when a task URL is available.
fn build_pr_body(body: &str, task_url: Option<&str>) -> String {
    match task_url {
        Some(url) if !body.contains(FOOTER_MARKER) => {
            // Reserve room for `\n\n---\n<marker> <url>\n`.
            let reserve = url.len() + FOOTER_MARKER.len() + 8;
            let max_body = MAX_COMMENT_LENGTH.saturating_sub(reserve);
            compose_footer(truncate_on_char_boundary(body, max_body), url)
        }
        _ => truncate_on_char_boundary(body, MAX_COMMENT_LENGTH).to_string(),
    }
}

/// Truncate a comment body to GitHub's limit and append the "view task" footer
/// when a task is associated. The user body is truncated first, reserving room
/// for the footer so the total stays within `MAX_COMMENT_LENGTH`.
async fn body_with_task_footer(
    tasks: &dyn TaskReader,
    frontend_url: &str,
    task_id: Option<&str>,
    body: &str,
) -> String {
    let footer = match task_id {
        Some(tid) => moatless_loops::bindings::footer::task_comment_footer(tasks, frontend_url, tid).await,
        None => None,
    };
    match footer {
        Some(f) => {
            let max_body = MAX_COMMENT_LENGTH.saturating_sub(f.len());
            format!("{}{}", truncate_on_char_boundary(body, max_body), f)
        }
        None => truncate_on_char_boundary(body, MAX_COMMENT_LENGTH).to_string(),
    }
}

/// Truncate to at most `max` bytes without splitting a UTF-8 character.
fn truncate_on_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// Upsert the task↔PR binding. Failure is logged and swallowed — the PR /
/// comment already exists on GitHub, so the mutation must still succeed
/// (mirrors the post-persist hooks behavior).
async fn bind_task_to_pr(
    adapter: &Arc<GithubPrAdapter>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    task_id: &str,
    user_id: &str,
) -> Option<String> {
    let (adapter_connections, loops, task_bindings) = match adapter.require_binding_ports() {
        Ok(ports) => ports,
        Err(error) => {
            warn!(task_id, pr_number, error = %error, "GitHub PR adapter missing binding ports");
            return None;
        }
    };
    // The adapter is the concrete GitHub PR destination builder the Connector
    // routes through — the same handle `ProviderHandles::github_destination_builder`
    // hands the server-band callers, sourced here from the provider we already hold.
    let github_destination: Arc<dyn AdapterDestinationBuilder> = adapter.clone();
    let source_id = format!("{owner}/{repo}");
    let thread_properties = json!({
        "owner": owner,
        "repo": repo,
        "pr_number": pr_number,
    });
    match moatless_loops::loops::firing::listen::Connector::new(
        Some(github_destination),
        adapter_connections,
        loops,
        task_bindings,
    )
    .connect_github_binding(&source_id, task_id, thread_properties, Some(user_id))
    .await
    {
        Ok((rule, binding)) => {
            info!(
                task_id,
                pr_number,
                subscription_id = rule.as_ref().map_or("<none>", |r| r.id.as_str()),
                binding_id = %binding.id,
                "Connected GitHub PR binding via typed API"
            );
            Some(binding.id)
        }
        Err(e) => {
            warn!(task_id, pr_number, error = %e, "Failed to connect GitHub PR binding via typed API");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate_on_char_boundary("hello", 10), "hello");
        assert_eq!(truncate_on_char_boundary("hello", 3), "hel");
        // 'é' is 2 bytes — truncating mid-char backs off to the boundary.
        assert_eq!(truncate_on_char_boundary("é", 1), "");
        assert_eq!(truncate_on_char_boundary("aé", 2), "a");
    }

    #[test]
    fn pr_body_includes_footer_when_task_url_present() {
        let out = build_pr_body("## Summary\nFix.", Some("https://x/tasks/t_1"));
        assert!(out.starts_with("## Summary"));
        assert!(out.contains("---\nMoatless task: https://x/tasks/t_1\n"));
    }

    #[test]
    fn pr_body_skips_footer_when_already_present() {
        let body = "body\n\n---\nMoatless task: https://x/tasks/t_1\n";
        let out = build_pr_body(body, Some("https://x/tasks/t_1"));
        assert_eq!(out, body);
    }

    #[test]
    fn pr_body_without_task_url_is_unchanged() {
        assert_eq!(build_pr_body("body", None), "body");
    }

    #[test]
    fn pr_body_truncates_before_appending_footer() {
        let url = "https://x/tasks/t_1";
        let long_body = "a".repeat(MAX_COMMENT_LENGTH + 100);
        let out = build_pr_body(&long_body, Some(url));
        assert!(out.len() <= MAX_COMMENT_LENGTH);
        assert!(out.contains(FOOTER_MARKER));
    }
}
