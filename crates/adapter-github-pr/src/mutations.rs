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

use moatless_adapter_model::binding::{BindReason, BindScope};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::footer::FOOTER_MARKER;
use moatless_loops::adapters::access::{ensure_task_access_only, ensure_task_access_with};
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_ports::task::{ExternalEvent, TaskDescriptor, TaskReader, TranscriptAppend};
use moatless_types::access_control::Actor;
use moatless_types::adapter::TaskBinding;
use moatless_types::attribution::Attribution;
use moatless_types::loops::SubscriptionMatch;

use super::adapter::GithubPrAdapter;
use super::api::MAX_COMMENT_LENGTH;
use super::auth::{resolve_task_github_api, resolve_task_github_writer, task_write_actor};
use super::external_id;
use super::reply_keys;
use super::subject::GithubPrSubject;
use super::text::{credit_driver, credit_pull_request, truncate_on_char_boundary};

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
    login: &str,
    (owner, repo): (String, String),
    req: GithubCreatePrRequest,
) -> Result<GithubCreatePrResponse, AdapterError> {
    let adapter = require_provider(github_pr_provider)?;
    let actor = task_actor(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;
    let writer = resolve_task_github_writer(&adapter, &actor, &owner, &repo).await?;
    let api = &writer.api;

    let base = match req.base.as_deref() {
        Some(b) => b.to_string(),
        None => api.get_repo_default_branch(&owner, &repo).await?,
    };

    let credits = adapter.auth().pull_request_credits(&actor.1).await;
    let body = build_pr_body(
        &credit_pull_request(
            &with_requester(
                req.body.as_deref().unwrap_or(""),
                actor
                    .0
                    .person()
                    .filter(|_| writer.credit().is_none())
                    .map(|_| login),
            ),
            &credits,
            writer.credit(),
        ),
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
        // A head branch with an already-open pull request isn't a failure:
        // GitHub answers 422, and we bind the existing pull request instead.
        // Any other create failure passes through unchanged.
        Err(create_error) => match api.find_open_pr_for_head(&owner, &repo, &req.head).await {
            Ok(Some(existing)) => CreatedPr::from_existing(&existing, &owner, &repo),
            _ => return Err(create_error),
        },
    };
    if let (Some(driver), false) = (writer.credit(), created.opened_existing) {
        hand_to_driver(api, &owner, &repo, created.number, driver).await;
    }

    let (binding_id, notified_events) = if let Some(task_id) = req.task_id.as_deref() {
        let bound = connect_pr_binding(
            connector.as_deref(),
            &owner,
            &repo,
            created.number,
            task_id,
            &ctx.user_id,
            &BindScope {
                close_on_subject_close: req.close_task_on_close,
                ..BindScope::because(BindReason::Creator)
            },
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
        match bound {
            Some((rule, binding)) => (
                Some(binding.id.clone()),
                super::routing::notified_events(rule.as_ref(), &binding),
            ),
            None => (None, Vec::new()),
        }
    } else {
        (None, Vec::new())
    };

    Ok(GithubCreatePrResponse {
        number: created.number,
        html_url: created.html_url,
        title: created.title,
        state: created.state,
        binding_id,
        opened_existing: created.opened_existing,
        notified_events,
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
    let actor = task_actor(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;
    let writer = resolve_task_github_writer(&adapter, &actor, &owner, &repo).await?;
    let api = &writer.api;

    let body = body_with_task_footer(
        tasks.as_ref(),
        adapter.frontend_url(),
        req.task_id.as_deref(),
        &credit_driver(&req.body, writer.credit()),
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
            &BindScope {
                reason: Some(BindReason::Commenter),
                reply_keys: reply_keys::opened_by_conversation_comment(comment_id, &req.body),
                close_on_subject_close: false,
            },
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
    let actor = task_actor(tasks.as_ref(), &ctx, req.task_id.as_deref()).await?;
    let writer = resolve_task_github_writer(&adapter, &actor, &owner, &repo).await?;
    let api = &writer.api;

    let body = body_with_task_footer(
        tasks.as_ref(),
        adapter.frontend_url(),
        req.task_id.as_deref(),
        &credit_driver(&req.body, writer.credit()),
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
            &BindScope {
                reason: Some(BindReason::Commenter),
                reply_keys: reply_keys::opened_by_review_comment(new_id, Some(comment_id)),
                close_on_subject_close: false,
            },
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
    let (actor, _) = task_actor(tasks.as_ref(), &ctx, Some(&req.task_id)).await?;
    let api = resolve_task_github_api(&adapter, &actor, &owner, &repo).await?;
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
        &BindScope::because(BindReason::Follower),
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

    let external_id = subject_external_id(&req)?;
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

/// The id a recorded subject is filed under; a [`GithubExternalEventSubject::Push`]
/// uses the push watcher's [`external_id::push`] claim, so `synchronize` aliases
/// onto it.
fn subject_external_id(req: &GithubExternalEventRequest) -> Result<String, AdapterError> {
    Ok(match &req.subject {
        GithubExternalEventSubject::ReviewComment(id) => external_id::review_comment(*id),
        GithubExternalEventSubject::Review(id) => external_id::review(*id),
        GithubExternalEventSubject::IssueComment(id) => external_id::issue_comment(*id),
        GithubExternalEventSubject::Push(sha) => {
            let (owner, repo) = req
                .source_name
                .as_deref()
                .and_then(|name| name.split_once('/'))
                .filter(|(owner, repo)| !owner.is_empty() && !repo.is_empty() && !sha.is_empty())
                .ok_or_else(|| {
                    SharedAdapterError::BadRequest(
                        "a push needs a sha and source_name `owner/repo`".into(),
                    )
                })?;
            external_id::push(owner, repo, sha)
        }
    })
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

/// Authorize the caller against the Task and answer the identity a write for it
/// acts as, per [`task_write_actor`], and who it credits.
///
/// Same `NotFound`-on-denied rule the routes always had; a route naming no Task
/// authorizes nothing and acts as the caller.
async fn task_actor(
    tasks: &dyn TaskReader,
    ctx: &moatless_types::access_control::ScopeContext,
    task_id: Option<&str>,
) -> Result<(Actor, Attribution), AdapterError> {
    let task = match task_id {
        Some(task_id) => Some(ensure_task_access_with(tasks, ctx, task_id).await?),
        None => None,
    };
    let actor = task_write_actor(
        &ctx.user_id,
        task.as_ref()
            .and_then(|task| task.run_as_user_id.as_deref()),
        task.as_ref().and_then(|task| task.workspace_id.as_deref()),
    );
    Ok((actor, task_attribution(&ctx.user_id, task.as_ref())))
}

fn task_attribution(caller: &str, task: Option<&TaskDescriptor>) -> Attribution {
    Attribution::of_caller(
        caller,
        task.and_then(|task| task.owner_user_id.as_deref().or(task.created_by.as_deref())),
        task.and_then(|task| task.reporter_user_id.as_deref()),
    )
}

/// Credit `requester` in a pull request body opened under someone else's
/// account; `None` when the caller opens it as themselves.
fn with_requester(body: &str, requester: Option<&str>) -> String {
    match requester {
        Some(login) => {
            let credit = format!("Requested by {login}");
            if body.contains(&credit) {
                body.to_string()
            } else if body.trim().is_empty() {
                credit
            } else {
                format!("{body}\n\n{credit}")
            }
        }
        None => body.to_string(),
    }
}

/// Assign `driver` to the pull request the App opened for them and request
/// their review, so GitHub itself shows whose it is. Best effort, each on its
/// own: the pull request already exists, and the body credits the driver.
async fn hand_to_driver(
    api: &super::api::GithubPrApi,
    owner: &str,
    repo: &str,
    number: u64,
    driver: &str,
) {
    let (assigned, review) = tokio::join!(
        api.add_assignee(owner, repo, number, driver),
        api.request_review(owner, repo, number, driver),
    );
    if let Err(e) = assigned {
        warn!(owner, repo, number, driver, error = %e, "Failed to assign the driver to the pull request");
    }
    if let Err(e) = review {
        warn!(owner, repo, number, driver, error = %e, "Failed to request the driver's review");
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
    scope: &BindScope,
) -> Option<String> {
    connect_pr_binding(connector, owner, repo, pr_number, task_id, user_id, scope)
        .await
        .map(|(_, binding)| binding.id)
}

/// The bound pull request's Subscription, when one matches, and the Binding.
/// `None` when nothing was bound; the cause is logged.
async fn connect_pr_binding(
    connector: Option<&moatless_loops::loops::firing::listen::Connector>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    task_id: &str,
    user_id: &str,
    scope: &BindScope,
) -> Option<(Option<SubscriptionMatch>, TaskBinding)> {
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
        .connect_github_binding(&source_id, task_id, thread_properties, Some(user_id), scope)
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
            Some((rule, binding))
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

    #[tokio::test]
    async fn the_driver_is_assigned_and_asked_to_review() {
        use super::super::test_support::{StubResponse, start_github_api_stub};
        use axum::http::{Method, StatusCode};

        let stub = start_github_api_stub(|method, path, uri| match (method, path) {
            (Method::POST, "/repos/acme/repo/issues/7/assignees")
            | (Method::POST, "/repos/acme/repo/pulls/7/requested_reviewers") => {
                StubResponse::json(StatusCode::CREATED, serde_json::json!({}))
            }
            _ => StubResponse::not_found(uri),
        })
        .await;
        let api = super::super::api::GithubPrApi::new(
            super::super::api::http_client(moatless_core::config::HttpConfig::default().github),
            "ghs_app",
            stub.base_url(),
        );

        hand_to_driver(&api, "acme", "repo", 7, "ada").await;

        let mut bodies: Vec<(String, serde_json::Value)> = stub
            .recorded_requests()
            .await
            .into_iter()
            .map(|r| (r.uri, serde_json::from_str(&r.body).unwrap()))
            .collect();
        bodies.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            bodies,
            vec![
                (
                    "/repos/acme/repo/issues/7/assignees".to_string(),
                    serde_json::json!({"assignees": ["ada"]})
                ),
                (
                    "/repos/acme/repo/pulls/7/requested_reviewers".to_string(),
                    serde_json::json!({"reviewers": ["ada"]})
                ),
            ]
        );
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
    fn a_pr_opened_for_someone_else_credits_them_once() {
        assert_eq!(
            with_requester("Fix.", Some("ada")),
            "Fix.\n\nRequested by ada"
        );
        assert_eq!(with_requester("", Some("ada")), "Requested by ada");
        let credited = "Fix.\n\nRequested by ada";
        assert_eq!(with_requester(credited, Some("ada")), credited);
    }

    #[test]
    fn a_pr_the_caller_opens_as_themselves_credits_nobody() {
        assert_eq!(with_requester("Fix.", None), "Fix.");
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

    /// A push made through the API is filed where a `synchronize` webhook for
    /// the same sha looks for the author; with no repository it is refused.
    #[test]
    fn a_recorded_push_is_filed_under_the_push_claim_synchronize_aliases() {
        let req = |source_name: Option<&str>| GithubExternalEventRequest {
            task_id: "t_1".into(),
            subject: GithubExternalEventSubject::Push("be1d882".into()),
            source_name: source_name.map(str::to_string),
        };

        let id = subject_external_id(&req(Some("acme/widgets"))).expect("a repository is named");
        assert!(external_id::aliases("github:pr:synchronize:acme/widgets#7:be1d882").contains(&id));
        assert!(subject_external_id(&req(None)).is_err());
        assert!(subject_external_id(&req(Some("acme"))).is_err());
    }
}
