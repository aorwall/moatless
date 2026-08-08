//! Comment routing: who on our side hears a comment written on a Linear issue.
//!
//! Two jobs, deliberately kept apart. Linear decides what *kind* of comment it
//! is — only Linear knows a `parentId` makes one a threaded reply — and that
//! classification is [`comment_audience`]. Turning an audience into recipients
//! is [`resolve_recipients`], which is shared with the GitHub PR Adapter and
//! knows nothing about Linear.
//!
//! The delivery a comment produces is the same one a person typing in the app
//! produces, so the two must not be told apart downstream: a comment the agent
//! itself posted arrives back through this path, and the only thing that stops
//! it becoming a message the agent answers is the external id matching what
//! `send_reply_for_binding` recorded.

use tracing::{debug, info, warn};

use moatless_core::config::Config;

use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, Audience};
use crate::bindings::recipients::{EventProvenance, resolve_recipients};
use crate::bindings::runtime::RoutingService;
use crate::loops::dao::LoopDao;
use crate::message::send::MessageSendService;
use crate::task::binding_dao::TaskBindingDao;
use crate::task::sandbox_waker::SandboxWaker;

use super::agent_api::{ActivityKind, LinearAgentApi};
use super::webhook_payloads::CommentPayloadData;

const ADAPTER_LABEL: &str = "linear";

/// Route a `Comment.create` payload to tasks listening to the comment's issue.
/// This includes both `linear_agent_session` bindings created when Linear
/// started an agent session and `linear_issue` bindings created by direct issue
/// sends/status-entry tasks. No recipient → no-op.
///
/// Which of those bindings receive the comment is not decided here: this
/// classifies the comment's [`Audience`] — a threaded reply names the comment it
/// replies to, anything else is for everyone on the issue — and
/// [`resolve_recipients`] turns that into recipients. The self-echo exclusion,
/// the thread scoping and the producer's standing to hear replies on its own
/// issue all come from there, so Linear and GitHub PR cannot drift apart on them
/// again.
///
/// The agent's own outbound replies are filtered twice over: the footer below
/// names the authoring task, and every reply posted via
/// `LinearAdapter::send_reply_for_binding` is recorded in
/// `AppState::recently_sent` and persisted to `task_messages.external_event_id`
/// under the key `linear:comment:<uuid>`. The inbound forward below builds the
/// same key, so `send_message_flow` also recognises the echo by comment id.
#[expect(
    clippy::too_many_arguments,
    reason = "the six downward ports this comment router reads (config, routing, \
              bindings, loops, message_send, sandbox_waker) inverted out of the \
              single AppState it used to carry; there is no narrower grouping \
              that is not itself an AppState-shaped bag"
)]
pub(super) async fn handle_comment_created(
    config: &Config,
    routing: &RoutingService,
    task_bindings: &dyn TaskBindingDao,
    loops: &LoopDao,
    message_send: &MessageSendService,
    sandbox_waker: &dyn SandboxWaker,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    issue_id: &str,
    comment: &CommentPayloadData,
) -> Result<(), AdapterError> {
    if comment.body.trim().is_empty() {
        debug!(comment_id = %comment.id, "Empty Comment.create body — ignoring");
        return Ok(());
    }

    // Provenance: our own outbound comments end with a `Moatless task:` footer
    // link to the authoring task (see `LinearAdapter::send_reply_for_binding`).
    // Recovering that task id is what lets the resolver drop the comment for the
    // task that authored it. The footer is visible content — nothing to strip,
    // so the body is forwarded unchanged.
    let author_task_id =
        crate::adapters::footer::parse_task_id_from_body(&config.auth.frontend_url, &comment.body);
    let forwarded_body = comment.body.as_str();

    let audience = comment_audience(comment);
    let parent_author_task_id = routing
        .parent_author_task_id(ADAPTER_LABEL, &audience)
        .await?;
    let bindings = super::subject::comment_recipients(task_bindings, connection, issue_id).await?;
    let recipients = resolve_recipients(
        &audience,
        &bindings,
        EventProvenance {
            author_task_id: author_task_id.as_deref(),
            parent_author_task_id: parent_author_task_id.as_deref(),
        },
    );

    if recipients.is_empty() {
        info!(
            issue_id,
            comment_id = %comment.id,
            parent_id = comment.parent_id.as_deref().unwrap_or(""),
            parent_author_task_id = parent_author_task_id.as_deref().unwrap_or(""),
            author_task_id = author_task_id.as_deref().unwrap_or(""),
            bound_tasks = bindings.len(),
            "Comment.create resolved to no Linear recipient — ignoring"
        );
        return Ok(());
    }

    let rule = super::subject::rule_for_binding(loops, recipients[0]).await?;
    let author = super::actor_handle(comment.user.as_ref());

    for binding in recipients {
        // Best-effort heartbeat into the bound agent session so Linear's
        // panel reflects that we're processing the comment. Ephemeral so
        // the next real activity replaces it in the UI.
        if let Some(session_id) = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
        {
            if let Err(e) = api
                .agent_activity_create(
                    session_id,
                    ActivityKind::Thought,
                    "Reading new comment…",
                    true,
                )
                .await
            {
                warn!(
                    session_id,
                    error = %e,
                    "Failed to emit Linear comment heartbeat"
                );
            }
        }

        let external_event_id = super::external_id::comment(&comment.id);
        if let Err(e) = message_send
            .send_message_flow(
                &binding.task_id,
                forwarded_body,
                None,
                None,
                None,
                &[],
                &[],
                true,
                crate::message::send::ExternalOrigin {
                    event_id: Some(&external_event_id),
                    adapter: Some(ADAPTER_LABEL),
                    subject_key: Some(&binding.subject_key),
                    source_name: rule.as_ref().map(|r| r.source_matcher.as_str()),
                    author: author.as_deref(),
                    source_task_id: None,
                },
                false,
                comment.user.as_ref().map(|u| u.id.as_str()),
                None,
            )
            .await
        {
            warn!(
                task_id = %binding.task_id,
                comment_id = %comment.id,
                error = %e,
                "Failed to forward Linear comment to task"
            );
            continue;
        }

        if let Err(e) = sandbox_waker.wake(&binding.task_id).await {
            warn!(
                task_id = %binding.task_id,
                error = %e,
                "Failed to ensure sandbox for Linear comment"
            );
        }
    }

    Ok(())
}

/// Who a `Comment.create` is for.
///
/// Linear makes exactly one distinction: a comment that carries a `parentId` is
/// a reply inside a thread, and belongs to whoever wrote the comment it replies
/// to — plus the task that produced the issue, which must hear the discussion on
/// its own work. Everything else is an ordinary issue-level comment for every
/// task bound to the issue.
///
/// A reply inside a thread no task wrote, on an issue no task produced, names
/// nobody at all; the resolver falls back to every bound task there, so the
/// common Linear shape — two humans threading under a loop-spawned task — keeps
/// reaching that task.
fn comment_audience(comment: &CommentPayloadData) -> Audience {
    match comment.parent_id.as_deref() {
        Some(parent_id) => Audience::ParentAuthor {
            parent_external_event_id: Some(super::external_id::comment(parent_id)),
        },
        None => Audience::Everyone,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::adapters::models::{ADAPTER_EVENT_BINDING_ORIGIN, TaskBinding};
    use crate::adapters::providers::linear::subject::{
        ADAPTER, SUBJECT_KIND_SESSION as SUBJECT_KIND,
    };

    // ---- Comment routing -------------------------------------------------
    //
    // Two halves, tested apart because they are two different jobs. The
    // classification below is Linear's: only Linear knows a `parentId` makes a
    // comment a threaded reply. Turning that into recipients is
    // `resolve_recipients`, shared with GitHub PR and exhaustively tabled in
    // `crate::bindings::recipients` — what the delivery cases here assert is
    // that Linear's own binding shapes fall where the ticket says they fall.

    fn comment_with_parent(parent_id: Option<&str>) -> CommentPayloadData {
        CommentPayloadData {
            id: "comment-1".into(),
            body: "Looks good, but see below.".into(),
            issue_id: Some("issue-1".into()),
            issue: None,
            user: None,
            parent_id: parent_id.map(str::to_string),
        }
    }

    #[test]
    fn an_issue_level_comment_is_for_everyone_on_the_issue() {
        assert_eq!(
            comment_audience(&comment_with_parent(None)),
            Audience::Everyone
        );
    }

    #[test]
    fn a_threaded_reply_names_the_comment_it_replies_to() {
        assert_eq!(
            comment_audience(&comment_with_parent(Some("comment-parent"))),
            Audience::ParentAuthor {
                parent_external_event_id: Some("linear:comment:comment-parent".into()),
            }
        );
    }

    fn linear_binding(task_id: &str, subject_kind: &str, origin: &str) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: format!("ts_{task_id}"),
            task_id: task_id.to_string(),
            adapter_kind: ADAPTER.to_string(),
            subject_kind: subject_kind.to_string(),
            subject_key: "issue-1".to_string(),
            connection_id: "pc_1".to_string(),
            loop_id: None,
            destination_config: serde_json::json!({ "issue_id": "issue-1" }),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: origin.to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        }
    }

    /// The task that filed the issue — the CLI-driven issue endpoint records
    /// `task_owned`, not the reply origin the other adapters use.
    fn issue_producer(task_id: &str) -> TaskBinding {
        linear_binding(
            task_id,
            crate::adapters::linear::SUBJECT_KIND_ISSUE,
            crate::adapters::models::TASK_OWNED_BINDING_ORIGIN,
        )
    }

    /// A task a Linear agent session spawned from the issue.
    fn session_responder(task_id: &str) -> TaskBinding {
        linear_binding(task_id, SUBJECT_KIND, ADAPTER_EVENT_BINDING_ORIGIN)
    }

    fn delivered_to(
        comment: &CommentPayloadData,
        bindings: &[TaskBinding],
        author_task_id: Option<&str>,
        parent_author_task_id: Option<&str>,
    ) -> Vec<String> {
        resolve_recipients(
            &comment_audience(comment),
            bindings,
            EventProvenance {
                author_task_id,
                parent_author_task_id,
            },
        )
        .into_iter()
        .map(|binding| binding.task_id.clone())
        .collect()
    }

    #[test]
    fn a_reply_to_a_responders_comment_reaches_it_and_the_issues_producer() {
        let bindings = vec![
            issue_producer("t_prod"),
            session_responder("t_resp"),
            session_responder("t_other"),
        ];
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-parent")),
                &bindings,
                None,
                Some("t_resp"),
            ),
            vec!["t_prod", "t_resp"],
            "the producer must hear a discussion on its own issue, \
             and no responder hears a thread it did not start"
        );
    }

    #[test]
    fn a_reply_on_an_issue_with_no_producer_reaches_the_parents_author_alone() {
        let bindings = vec![session_responder("t_resp"), session_responder("t_other")];
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-parent")),
                &bindings,
                None,
                Some("t_resp"),
            ),
            vec!["t_resp"],
            "most Linear issues have no producer, and those route exactly as before"
        );
    }

    #[test]
    fn a_reply_inside_a_thread_no_task_wrote_still_reaches_a_producerless_issue() {
        let bindings = vec![session_responder("t_resp"), session_responder("t_other")];
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-parent")),
                &bindings,
                None,
                None,
            ),
            vec!["t_resp", "t_other"],
            "two humans threading under a loop-spawned task name nobody, \
             and the resolver's fallback keeps the task hearing it"
        );
    }

    #[test]
    fn an_issue_level_comment_still_reaches_every_bound_task() {
        let bindings = vec![
            issue_producer("t_prod"),
            session_responder("t_resp"),
            session_responder("t_other"),
        ];
        assert_eq!(
            delivered_to(&comment_with_parent(None), &bindings, None, None),
            vec!["t_prod", "t_resp", "t_other"]
        );
    }

    #[test]
    fn a_task_never_receives_back_the_comment_it_posted() {
        let bindings = vec![issue_producer("t_prod"), session_responder("t_resp")];
        assert_eq!(
            delivered_to(&comment_with_parent(None), &bindings, Some("t_resp"), None),
            vec!["t_prod"],
            "the footer names the authoring task; only that task is dropped"
        );
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-parent")),
                &bindings,
                Some("t_prod"),
                Some("t_resp"),
            ),
            vec!["t_resp"],
            "a producer replying inside a responder's thread does not hear itself"
        );
    }
}
