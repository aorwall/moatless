//! Comment routing: who on our side hears a comment written on a Linear issue.
//!
//! A binding either owns its whole ticket or is scoped to one comment thread;
//! [`hears_comment`] decides which comments reach it.
//!
//! The agent's own posted comments arrive back through this same webhook path.
//! Two records keep them from being replayed as new messages: the footer naming
//! the authoring task, and the external event
//! `issue_commands::note_comment_as_own` wrote.

use tracing::{debug, info, warn};

use moatless_core::config::Config;

use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::bindings::operations::Bindings;
use moatless_loops::bindings::runtime::RoutingService;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::{InboundTurn, MessageAcceptor, TurnOrigin};
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::Audience;
use moatless_types::adapter::TaskBinding;

use super::agent_api::{ActivityKind, LinearAgentApi};
use super::subject::{SUBJECT_KIND_ISSUE, SUBJECT_KIND_SESSION, origin_comment_id};
use super::webhook_payloads::CommentPayloadData;

const ADAPTER_LABEL: &str = moatless_types::adapter::AdapterKind::Linear.as_str();

/// Route a `Comment.create` payload to the bound tasks listening on its issue.
/// No recipient → no-op.
///
/// Own replies are excluded twice: the footer names the authoring task, and
/// `send_reply_for_binding` records each reply's id in
/// `task_outbound_events.external_event_id`, the key the forward below builds.
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
    task_bindings: &Bindings,
    loops: &dyn LoopReader,
    message_send: &dyn MessageAcceptor,
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

    // Our own outbound comments end with a `Moatless task:` footer link (see
    // `LinearAdapter::send_reply_for_binding`); recovering that id is how the
    // resolver drops the comment for its own authoring task. The footer is
    // visible, so the body is forwarded unchanged.
    let author_task_id = moatless_adapter_model::footer::parse_task_id_from_body(
        &config.auth.frontend_url,
        &comment.body,
    );
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
    let author = super::webhook_payloads::actor_handle(comment.user.as_ref());

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
            .accept(InboundTurn {
                task_id: binding.task_id.clone(),
                text: forwarded_body.to_string(),
                file_ids: Vec::new(),
                resume: true,
                origin: TurnOrigin {
                    event_id: Some(external_event_id.clone()),
                    adapter: Some(ADAPTER_LABEL.to_string()),
                    subject_key: Some(binding.subject_key.clone()),
                    source_name: rule.as_ref().map(|r| r.source_matcher.clone()),
                    author: author.clone(),
                    sender_id: None,
                },
                sent_by: comment.user.as_ref().map(|u| u.id.clone()),
                skills: Vec::new(),
            })
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
/// `author_task_id` is excluded first, so a task never hears back what it
/// authored; the rest is [`hears_comment`].
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
        .filter(|binding| hears_comment(binding, comment_parent_id, parent_author_task_id))
        .filter(|binding| seen_tasks.insert(binding.task_id.clone()))
        .collect()
}

/// Whether one binding is listening where this comment was written.
///
/// A thread-scoped binding (any session binding, or an issue binding carrying
/// an `origin_comment_id`) hears a reply to a comment its own task authored or
/// to the comment it was scoped to. An unscoped `linear_issue` binding owns the
/// ticket, so every comment on it is for that task.
fn hears_comment(
    binding: &TaskBinding,
    comment_parent_id: Option<&str>,
    parent_author_task_id: Option<&str>,
) -> bool {
    let is_issue_binding = binding.subject_kind == SUBJECT_KIND_ISSUE;
    if !is_issue_binding && binding.subject_kind != SUBJECT_KIND_SESSION {
        return false;
    }

    let scoped_to = origin_comment_id(binding);
    if is_issue_binding && scoped_to.is_none() {
        return true;
    }

    let replies_to_own_comment = parent_author_task_id == Some(binding.task_id.as_str());
    let replies_to_origin_comment = comment_parent_id.is_some() && comment_parent_id == scoped_to;
    replies_to_own_comment || replies_to_origin_comment
}

/// Who a `Comment.create` is for: a comment with a `parentId` is a threaded
/// reply and belongs to whoever wrote the comment it replies to; otherwise it's
/// for everyone on the issue.
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
    use moatless_types::adapter::AdapterKind;

    use super::*;
    use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
    use moatless_types::adapter::TaskBinding;

    // Comment routing: two jobs, tested apart. Classification (`parentId` →
    // threaded reply) is Linear-specific; turning that into recipients is
    // `resolve_recipients`, shared with GitHub PR and tabled exhaustively in
    // `moatless_loops::bindings::recipients`. These tests only assert Linear's
    // binding shapes land where that table says.

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
            watching: true,
        }
    }

    /// The task that filed the issue — the CLI-driven issue endpoint records
    /// `task_owned`, not the reply origin the other adapters use.
    fn issue_producer(task_id: &str) -> TaskBinding {
        linear_binding(
            task_id,
            SUBJECT_KIND_ISSUE,
            moatless_adapter_model::binding::TASK_OWNED_BINDING_ORIGIN,
        )
    }

    /// A task a Linear agent session spawned from the issue, with no recorded
    /// originating comment (an assignment-initiated session).
    fn session_responder(task_id: &str) -> TaskBinding {
        linear_binding(task_id, SUBJECT_KIND_SESSION, ADAPTER_EVENT_BINDING_ORIGIN)
    }

    /// A session task spawned from a mention on `origin_comment_id`.
    fn session_responder_from(task_id: &str, origin_comment_id: &str) -> TaskBinding {
        scoped_to(session_responder(task_id), origin_comment_id)
    }

    /// The task that posted `origin_comment_id` on an issue it does not own —
    /// what `linear issue comment` writes without `--bind`.
    fn comment_author(task_id: &str, origin_comment_id: &str) -> TaskBinding {
        scoped_to(issue_producer(task_id), origin_comment_id)
    }

    fn scoped_to(mut binding: TaskBinding, origin_comment_id: &str) -> TaskBinding {
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
    fn a_reply_to_a_comment_a_task_posted_reaches_that_task() {
        let bindings = vec![comment_author("t_author", "comment-mine")];
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-mine")),
                &bindings,
                None,
                // The durable record of the comment is gone (or never landed),
                // so only the binding's own scope can match.
                None,
            ),
            vec!["t_author"],
            "the comment a task posted is its subject: a reply under it comes back"
        );
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-later")),
                &bindings,
                None,
                Some("t_author"),
            ),
            vec!["t_author"],
            "and a reply under any later comment the same task posted does too"
        );
    }

    #[test]
    fn a_comment_outside_a_task_authored_thread_does_not_reach_it() {
        let bindings = vec![comment_author("t_author", "comment-mine")];
        assert_eq!(
            delivered_to(&comment_with_parent(None), &bindings, None, None),
            Vec::<String>::new(),
            "a task that only commented on someone else's ticket does not hear \
             unrelated chatter on it"
        );
        assert_eq!(
            delivered_to(
                &comment_with_parent(Some("comment-theirs")),
                &bindings,
                None,
                None,
            ),
            Vec::<String>::new(),
            "nor a reply in a thread it never wrote in"
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
