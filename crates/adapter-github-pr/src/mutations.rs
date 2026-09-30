//! Authorized provider operations and ordered mutation effects.

use std::fmt::Write as _;
use std::sync::Arc;

use moatless_api_schemas::github::{
    GithubBindPrRequest, GithubBindPrResponse, GithubCommentRequest, GithubCommentResponse,
    GithubCreatePrRequest, GithubCreatePrResponse, GithubExternalEventRequest,
    GithubExternalEventResponse, GithubExternalEventSubject, GithubReplyRequest,
};
use serde_json::{Value, json};
use tracing::{info, warn};

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::footer::FOOTER_MARKER;
use moatless_loops::adapters::access::{ensure_task_access_only, ensure_task_access_with};
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_ports::task::{ExternalEvent, TaskReader, TranscriptAppend};

use super::adapter::GithubPrAdapter;
use super::api::MAX_COMMENT_LENGTH;
use super::auth::resolve_github_api;
use super::external_id;
use super::subject::GithubPrSubject;
use super::text::truncate_on_char_boundary;

/// Provider label recorded on an external event, and the `external_adapter` an
/// inbound event is matched under.
const ADAPTER_LABEL: &str = moatless_types::adapter::AdapterKind::GithubPr.as_str();

fn require_provider(
    provider: Option<Arc<GithubPrAdapter>>,
) -> Result<Arc<GithubPrAdapter>, AdapterError> {
    provider.ok_or_else(|| SharedAdapterError::AdapterNotConfigured("github_pr".into()).into())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the authorized mutation receives separate provider, binding, transcript, task, and caller dependencies"
)]
pub(crate) async fn create_pr(
    connector: Option<Arc<moatless_loops::loops::firing::listen::Connector>>,
    github_pr_provider: Option<Arc<GithubPrAdapter>>,
    tasks: Arc<dyn TaskReader>,
    recently_sent: Arc<RecentlySent>,
    messages: Arc<dyn TranscriptAppend>,
    ctx: moatless_types::access_control::ScopeContext,
    (owner, repo): (String, String),
    req: GithubCreatePrRequest,
) -> Result<GithubCreatePrResponse, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let workspace_id = task_workspace(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;

    let api = resolve_github_api(
        &adapter,
        &ctx.user_id,
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
            connector.as_deref(),
            &owner,
            &repo,
            created.number,
            task_id,
            &ctx.user_id,
        )
        .await;
        note_external_event(
            recently_sent.as_ref(),
            messages.as_ref(),
            task_id,
            &external_id::pr_opened(&GithubPrSubject::new(&owner, &repo, created.number)),
            &owner,
            &repo,
        )
        .await;
        bid
    } else {
        None
    };

    Ok(GithubCreatePrResponse {
        number: created.number,
        html_url: created.html_url,
        title: created.title,
        state: created.state,
        binding_id,
        opened_existing: created.opened_existing,
    })
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

#[expect(
    clippy::too_many_arguments,
    reason = "the authorized mutation receives separate provider, binding, transcript, task, and caller dependencies"
)]
pub(crate) async fn post_comment(
    connector: Option<Arc<moatless_loops::loops::firing::listen::Connector>>,
    github_pr_provider: Option<Arc<GithubPrAdapter>>,
    tasks: Arc<dyn TaskReader>,
    recently_sent: Arc<RecentlySent>,
    messages: Arc<dyn TranscriptAppend>,
    ctx: moatless_types::access_control::ScopeContext,
    (owner, repo, number): (String, String, u64),
    req: GithubCommentRequest,
) -> Result<GithubCommentResponse, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let workspace_id = task_workspace(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;

    let api = resolve_github_api(
        &adapter,
        &ctx.user_id,
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
            connector.as_deref(),
            &owner,
            &repo,
            number,
            task_id,
            &ctx.user_id,
        )
        .await
    } else {
        None
    };

    // GitHub redirects `/issues/<n>#issuecomment-<id>` to `/pull/<n>` for PRs,
    // so this URL form works for both PRs and issues without an extra fetch.
    let html_url =
        format!("https://github.com/{owner}/{repo}/issues/{number}#issuecomment-{comment_id}");

    Ok(GithubCommentResponse {
        id: comment_id,
        html_url,
        binding_id,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the authorized mutation receives separate provider, binding, transcript, task, and caller dependencies"
)]
pub(crate) async fn reply_to_review_comment(
    connector: Option<Arc<moatless_loops::loops::firing::listen::Connector>>,
    github_pr_provider: Option<Arc<GithubPrAdapter>>,
    tasks: Arc<dyn TaskReader>,
    recently_sent: Arc<RecentlySent>,
    messages: Arc<dyn TranscriptAppend>,
    ctx: moatless_types::access_control::ScopeContext,
    (owner, repo, number, comment_id): (String, String, u64, u64),
    req: GithubReplyRequest,
) -> Result<GithubCommentResponse, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let workspace_id = task_workspace(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;

    let api = resolve_github_api(
        &adapter,
        &ctx.user_id,
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
            connector.as_deref(),
            &owner,
            &repo,
            number,
            task_id,
            &ctx.user_id,
        )
        .await
    } else {
        None
    };

    let html_url = format!("https://github.com/{owner}/{repo}/pull/{number}#discussion_r{new_id}");

    Ok(GithubCommentResponse {
        id: new_id,
        html_url,
        binding_id,
    })
}

pub(crate) async fn bind_pr(
    connector: Option<Arc<moatless_loops::loops::firing::listen::Connector>>,
    github_pr_provider: Option<Arc<GithubPrAdapter>>,
    tasks: Arc<dyn TaskReader>,
    ctx: moatless_types::access_control::ScopeContext,
    (owner, repo, number): (String, String, u64),
    req: GithubBindPrRequest,
) -> Result<GithubBindPrResponse, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let workspace_id = task_workspace(tasks.as_ref(), &ctx, Some(&req.task_id)).await?;

    let api = resolve_github_api(
        &adapter,
        &ctx.user_id,
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
        connector.as_deref(),
        &owner,
        &repo,
        number,
        &req.task_id,
        &ctx.user_id,
    )
    .await;

    Ok(GithubBindPrResponse {
        number,
        html_url,
        binding_id,
    })
}

/// Record a GitHub event the caller produced outside the typed routes above.
///
/// `moat gh api` reaches GitHub directly (its endpoint surface is too wide to
/// type), so the caller must self-report the resulting event here instead of it
/// being recorded inline.
pub(crate) async fn record_external_event(
    tasks: Arc<dyn TaskReader>,
    recently_sent: Arc<RecentlySent>,
    messages: Arc<dyn TranscriptAppend>,
    ctx: moatless_types::access_control::ScopeContext,
    req: GithubExternalEventRequest,
) -> Result<GithubExternalEventResponse, AdapterError> {
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
    Ok(GithubExternalEventResponse { recorded })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Append the `Moatless task: <url>` footer to a body.
///
/// Baked in at creation so the link exists from the first revision, letting the
/// inbound echo filter recover the authoring task from any comment quoting it.
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
/// commits; the durable row still answers after a restart or on a replica that
/// didn't serve this request.
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
/// Best effort: a failure costs one echoed event on a replica that didn't serve
/// this request, since the in-memory note already landed here.
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
/// Same `NotFound`-on-denied rule the routes always had; a route naming no Task
/// authorizes nothing and has no Workspace to report.
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

/// Compose the PR body, truncated to GitHub's limit, with the `Moatless task:`
/// footer baked in when a task URL is available.
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
        Some(tid) => {
            moatless_loops::bindings::footer::task_comment_footer(tasks, frontend_url, tid).await
        }
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

/// Upsert the task↔PR binding. Failure is logged and swallowed — the PR /
/// comment already exists on GitHub, so the mutation must still succeed
/// (mirrors the post-persist hooks behavior).
async fn bind_task_to_pr(
    connector: Option<&moatless_loops::loops::firing::listen::Connector>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    task_id: &str,
    user_id: &str,
) -> Option<String> {
    let Some(connector) = connector else {
        warn!(
            task_id,
            pr_number, "GitHub PR adapter missing binding connector"
        );
        return None;
    };
    let source_id = format!("{owner}/{repo}");
    let thread_properties = json!({
        "owner": owner,
        "repo": repo,
        "pr_number": pr_number,
    });
    match connector
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
