//! Comment routing: who on our side hears a comment written on a Linear issue.
//!
//! Recipients follow the binding's shape ([`recipients_for_comment`]):
//! - A `linear_issue` binding is the task that filed or owns the ticket; it
//!   hears every comment on the issue.
//! - A `linear_agent_session` binding is a task a mention or assignment spawned
//!   into the ticket; it is thread-scoped. It hears a comment only when the
//!   comment replies to a comment it authored or to the comment the session was
//!   created from. It never hears an issue-level comment or another session's
//!   thread — so two sessions on one issue cannot ferry their own `response`
//!   comments back and forth forever.
//!
//! The delivery a comment produces is the same one a person typing in the app
//! produces, so the two must not be told apart downstream: a comment the agent
//! itself posted arrives back through this path, and the thing that stops it
//! becoming a message the agent answers is the footer naming the authoring task
//! (dropped here) plus the external id matching what `send_reply_for_binding`
//! recorded.

use tracing::{debug, info, warn};

use moatless_core::config::Config;

use moatless_loops::adapters::error::AdapterError;
use moatless_loops::adapters::models::{AdapterConnection, TaskBinding};
use moatless_types::adapter::Audience;
use moatless_loops::bindings::runtime::RoutingService;
use moatless_loops::loops::dao::LoopDao;
use moatless_tasks::message::send::MessageSendService;
use moatless_tasks::task::binding_dao::TaskBindingDao;
use moatless_tasks::task::sandbox_waker::SandboxWaker;

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
        moatless_loops::adapters::footer::parse_task_id_from_body(&config.auth.frontend_url, &comment.body);
    let forwarded_body = comment.body.as_str();

    let audience = comment_audience(comment);
    let parent_author_task_id = routing
        .parent_author_task_id(ADAPTER_LABEL, &audience)
        .await?;
    let bindings =
        super::subject::active_comment_bindings(task_bindings, connection, issue_id).await?;
    let recipients = recipients_for_comment(
        &bindings,
        comment.parent_id.as_deref(),
        author_task_id.as_deref(),
        parent_author_task_id.as_deref(),
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
                moatless_types::task::ExternalOrigin {
                    event_id: Some(&external_event_id),
                    adapter: Some(ADAPTER_LABEL),
                    subject_key: Some(&binding.subject_key),
                    source_name: rule.as_ref().map(|r| r.source_matcher.as_str()),
                    author: author.as_deref(),
                    source_task_id: None,
                },
                false,
                comment.user.as_ref().map(|u| u.id.as_str()),
                &[],
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

/// The bound tasks a `Comment.create` is delivered to, scoped by binding shape.
///
/// A `linear_issue` binding — the task that filed or owns the ticket — hears
/// every comment on the issue. A `linear_agent_session` binding — a task a
/// mention or assignment spawned into the ticket — is thread-scoped: it hears a
/// comment only when the comment replies to a comment the task authored
/// (`parent_author_task_id`) or to the comment the session was created from
/// (`origin_comment_id`, recorded on the binding at provisioning). It never
/// hears an issue-level comment or another session's thread, which is what keeps
/// two sessions on one issue from ferrying their own `response` comments to each
/// other forever.
///
/// A task never receives back a comment it authored: `author_task_id` is
/// recovered from the footer and excluded first. At most one Binding per task is
/// returned; `bindings` arrives session-first, so a task whose session matched
/// is reached through it (and its panel heartbeats), while a task reached only
/// through its issue Binding keeps that one.
fn recipients_for_comment<'b>(
    bindings: &'b [TaskBinding],
    comment_parent_id: Option<&str>,
    author_task_id: Option<&str>,
    parent_author_task_id: Option<&str>,
) -> Vec<&'b TaskBinding> {
    let mut seen_tasks = std::collections::HashSet::new();
    bindings
        .iter()
        .filter(|binding| author_task_id != Some(binding.task_id.as_str()))
        .filter(|binding| {
            if binding.subject_kind == crate::SUBJECT_KIND_ISSUE {
                return true;
            }
            if binding.subject_kind == crate::subject::SUBJECT_KIND_SESSION {
                let replies_to_own_comment =
                    parent_author_task_id == Some(binding.task_id.as_str());
                let origin_comment_id = binding
                    .destination_config
                    .get("origin_comment_id")
                    .and_then(serde_json::Value::as_str);
                let replies_to_origin_comment =
                    comment_parent_id.is_some() && comment_parent_id == origin_comment_id;
                return replies_to_own_comment || replies_to_origin_comment;
            }
            false
        })
        .filter(|binding| seen_tasks.insert(binding.task_id.clone()))
        .collect()
}

/// Who a `Comment.create` is for.
///
/// Linear makes exactly one distinction that this classifies: a comment that
/// carries a `parentId` is a reply inside a thread, and belongs to whoever wrote
/// the comment it replies to. The resulting [`Audience::ParentAuthor`] is what
/// [`RoutingService::parent_author_task_id`] resolves the reply's parent author
/// from; the binding-shape scoping that turns that into recipients lives in
/// [`recipients_for_comment`].
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
    use moatless_types::adapter::AdapterKind;
    use chrono::Utc;

    use super::*;
    use moatless_loops::adapters::models::{ADAPTER_EVENT_BINDING_ORIGIN, TaskBinding};
    use crate::subject::SUBJECT_KIND_SESSION as SUBJECT_KIND;

    // ---- Comment routing -------------------------------------------------
    //
    // Two halves, tested apart because they are two different jobs. The
    // classification below is Linear's: only Linear knows a `parentId` makes a
    // comment a threaded reply. Turning that into recipients is
    // `resolve_recipients`, shared with GitHub PR and exhaustively tabled in
    // `moatless_loops::bindings::recipients` — what the delivery cases here assert is
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
            adapter_kind: AdapterKind::Linear,
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
            crate::SUBJECT_KIND_ISSUE,
            moatless_loops::adapters::models::TASK_OWNED_BINDING_ORIGIN,
        )
    }

    /// A task a Linear agent session spawned from the issue, with no recorded
    /// originating comment (an assignment-initiated session).
    fn session_responder(task_id: &str) -> TaskBinding {
        linear_binding(task_id, SUBJECT_KIND, ADAPTER_EVENT_BINDING_ORIGIN)
    }

    /// A session task spawned from a mention on `origin_comment_id`.
    fn session_responder_from(task_id: &str, origin_comment_id: &str) -> TaskBinding {
        let mut binding = session_responder(task_id);
        binding.destination_config = serde_json::json!({
            "issue_id": "issue-1",
            "origin_comment_id": origin_comment_id,
        });
        binding
    }

    fn delivered_to(
        comment: &CommentPayloadData,
        bindings: &[TaskBinding],
        author_task_id: Option<&str>,
        parent_author_task_id: Option<&str>,
    ) -> Vec<String> {
        recipients_for_comment(
            bindings,
            comment.parent_id.as_deref(),
            author_task_id,
            parent_author_task_id,
        )
        .into_iter()
        .map(|binding| binding.task_id.clone())
        .collect()
    }

    #[test]
    fn a_reply_to_a_sessions_comment_reaches_it_and_the_issues_creator() {
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
            "the ticket creator hears every comment, and a session hears a reply \
             to a comment it authored — but no sibling session hears a thread it \
             did not start"
        );
    }

    #[test]
    fn a_reply_to_a_sessions_origin_comment_reaches_that_session() {
        let bindings = vec![
            session_responder_from("t_resp", "comment-parent"),
            session_responder_from("t_other", "some-other-comment"),
        ];
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-parent")),
                &bindings,
                None,
                // The origin comment was written by the human who mentioned the
                // agent, so no task authored the parent.
                None,
            ),
            vec!["t_resp"],
            "a reply to the comment a session was created from reaches that \
             session, and only it"
        );
    }

    #[test]
    fn an_issue_level_comment_reaches_only_the_ticket_creator() {
        let bindings = vec![
            issue_producer("t_prod"),
            session_responder("t_resp"),
            session_responder("t_other"),
        ];
        assert_eq!(
            delivered_to(&comment_with_parent(None), &bindings, None, None),
            vec!["t_prod"],
            "a top-level issue comment is for the task that owns the ticket; a \
             session hears only its own thread"
        );
    }

    #[test]
    fn an_issue_level_comment_with_no_creator_reaches_nobody() {
        let bindings = vec![session_responder("t_resp"), session_responder("t_other")];
        assert_eq!(
            delivered_to(&comment_with_parent(None), &bindings, None, None),
            Vec::<String>::new(),
            "a session does not hear a comment outside its own thread, so an \
             issue with only sessions and no creator hears an issue-level \
             comment nowhere"
        );
    }

    #[test]
    fn a_reply_in_a_thread_a_session_did_not_start_reaches_nobody() {
        let bindings = vec![session_responder("t_resp"), session_responder("t_other")];
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-parent")),
                &bindings,
                None,
                None,
            ),
            Vec::<String>::new(),
            "a reply to a comment no session authored and that no session was \
             created from reaches no session"
        );
    }

    #[test]
    fn two_sessions_on_one_issue_do_not_hear_each_others_responses() {
        // The production loop: two agent-session peers on one issue. Each posts
        // its answer as a `response`, which Linear mirrors back as a top-level
        // issue comment authored by that peer. Neither peer hears the other's,
        // so the ping-pong cannot start.
        let bindings = vec![
            session_responder_from("t_a", "comment-a"),
            session_responder_from("t_b", "comment-b"),
        ];
        assert_eq!(
            delivered_to(&comment_with_parent(None), &bindings, Some("t_a"), None),
            Vec::<String>::new(),
            "peer t_b does not hear t_a's issue-level response"
        );
        assert_eq!(
            delivered_to(&comment_with_parent(None), &bindings, Some("t_b"), None),
            Vec::<String>::new(),
            "and t_a does not hear t_b's"
        );
    }

    #[test]
    fn a_task_never_receives_back_the_comment_it_posted() {
        let bindings = vec![issue_producer("t_prod"), session_responder("t_resp")];
        assert_eq!(
            delivered_to(&comment_with_parent(None), &bindings, Some("t_resp"), None),
            vec!["t_prod"],
            "the footer names the authoring task; only that task is dropped, and \
             the ticket creator still hears the comment"
        );
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-parent")),
                &bindings,
                Some("t_prod"),
                Some("t_resp"),
            ),
            vec!["t_resp"],
            "a creator replying inside a session's thread does not hear itself"
        );
    }
}
