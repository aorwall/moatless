//! What a Linear Binding points at, and how to find one.
//!
//! Two subject kinds, and the difference is not cosmetic. A
//! `linear_agent_session` Binding names a Linear agent session — it has a panel
//! that can be kept alive with an activity, and replies land there. A
//! `linear_issue` Binding names an issue with no session behind it, created by
//! the CLI, by an operator, or by an issue entering a trigger state; it has no
//! panel, so it costs no activity call and hears only comments.
//!
//! Both carry the issue id in `destination_config`, which is what lets one
//! comment reach a task bound either way. That is also why the two
//! `destination_config` literals live here rather than at the two provisioning
//! sites: `issue_id` is a lookup key, and a provisioning path that spelled it
//! differently would write a Binding nothing could find.

use moatless_adapter_model::error::AdapterError;
use moatless_types::adapter::AdapterConnection;
use moatless_types::loops::SubscriptionMatch;
use moatless_types::adapter::TaskBinding;
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::bindings::dao::TaskBindingDao;

use super::SUBJECT_KIND_ISSUE;
use super::webhook_payloads::{AgentSession, IssuePayload};

pub(super) const ADAPTER: &str = "linear";
/// `task_bindings.subject_kind` for a Binding that names a Linear agent
/// session. Its `subject_key` is the session id.
pub(super) const SUBJECT_KIND_SESSION: &str = "linear_agent_session";

/// Which Bindings a lookup is asking for.
///
/// The two shapes differ only in which key they filter on, which is why they
/// are one lookup with a parameter rather than two: everything else about them
/// — the adapter, the subject kinds, the pending guard — has to stay in step.
pub(super) enum BindingScope<'a> {
    /// Every Binding of either subject kind pointing at this issue. Used by the
    /// close path, which must consider a task bound through a session and a
    /// task bound directly to the issue alike.
    OnIssue(&'a str),
    /// Every session Binding of this task. Used by the activity emitters, which
    /// have a panel to talk to only where a session exists.
    SessionsOfTask(&'a str),
}

/// The Bindings `scope` names, in no particular order.
///
/// Both shapes skip a Binding its `destination_config` still marks pending: a
/// provisioning path writes one before the Linear side exists, and neither
/// caller may act on a subject that does not exist yet, so the guard is part of
/// this lookup rather than left to each caller to remember.
///
/// Note this deliberately does *not* filter on `active`: the close path needs
/// to see a Binding in order to decide about it, and the emitters' callers
/// filter for themselves. Callers that want only live Bindings say so.
pub(super) async fn bindings_in_scope(
    task_bindings: &dyn TaskBindingDao,
    scope: BindingScope<'_>,
) -> Result<Vec<TaskBinding>, AdapterError> {
    match scope {
        BindingScope::OnIssue(issue_id) => {
            let mut rows = Vec::new();
            for subject_kind in [SUBJECT_KIND_SESSION, SUBJECT_KIND_ISSUE] {
                rows.extend(
                    task_bindings
                        .find_active_by_destination_field(
                            ADAPTER,
                            subject_kind,
                            "issue_id",
                            issue_id,
                        )
                        .await?,
                );
            }
            Ok(rows)
        }
        BindingScope::SessionsOfTask(task_id) => Ok(task_bindings
            .find_by_task_id(task_id)
            .await?
            .into_iter()
            .filter(|binding| {
                binding.adapter_kind.as_str() == ADAPTER
                    && binding.subject_kind == SUBJECT_KIND_SESSION
                    && !is_pending(binding)
            })
            .collect()),
    }
}

/// Whether a provisioning path wrote this Binding before its Linear subject
/// existed.
fn is_pending(binding: &TaskBinding) -> bool {
    binding
        .destination_config
        .get("pending")
        .and_then(serde_json::Value::as_str)
        == Some("true")
}

/// Every active Binding of either subject kind listening to `issue_id`, scoped
/// to `connection`, session Bindings first.
///
/// Not de-duplicated per task. Which of a task's Bindings hears a given comment
/// depends on the comment — an issue Binding hears every comment on the issue, a
/// session Binding only its own thread (see `comments::recipients_for_comment`)
/// — and the panel heartbeat wants the session Binding specifically. The
/// session-first order lets the recipient step keep the session Binding whenever
/// a task's session is the one that matched.
pub(super) async fn active_comment_bindings(
    task_bindings: &dyn TaskBindingDao,
    connection: &AdapterConnection,
    issue_id: &str,
) -> Result<Vec<TaskBinding>, AdapterError> {
    let mut rows = Vec::new();
    for subject_kind in [SUBJECT_KIND_SESSION, SUBJECT_KIND_ISSUE] {
        rows.extend(
            task_bindings
                .find_active_by_destination_field(ADAPTER, subject_kind, "issue_id", issue_id)
                .await?,
        );
    }

    // Skip bindings whose task has closed — a closed task is terminal and must
    // not be resurrected by a new comment.
    Ok(rows
        .into_iter()
        .filter(|b| b.connection_id == connection.id && b.active)
        .collect())
}

/// Active Bindings of `subject_kind` pointing at `issue_id`, scoped to
/// `connection`.
pub(super) async fn active_bindings_on_issue(
    task_bindings: &dyn TaskBindingDao,
    connection: &AdapterConnection,
    subject_kind: &str,
    issue_id: &str,
) -> Result<Vec<TaskBinding>, AdapterError> {
    let rows = task_bindings
        .find_active_by_destination_field(ADAPTER, subject_kind, "issue_id", issue_id)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|b| b.connection_id == connection.id)
        .collect())
}

/// The Subscription a Binding was created by, if it still exists.
///
/// A Binding whose Subscription has since been deleted keeps working —
/// `None` here means "no Subscription context", not "stop". Existing Bindings
/// keep receiving events; only new trigger-created Task behaviour is gated on
/// the Subscription.
pub(super) async fn rule_for_binding(
    loops: &dyn LoopReader,
    binding: &TaskBinding,
) -> Result<Option<SubscriptionMatch>, AdapterError> {
    match binding.loop_id {
        Some(ref loop_id) => Ok(loops.subscription(loop_id).await?),
        None => Ok(None),
    }
}

/// The `destination_config` of a Binding created from a Linear agent session.
///
/// `session_id` is what makes it a session Binding — the reply path and the
/// heartbeat both read it, and its absence is what tells the two kinds apart at
/// a call site that only has the JSON.
pub(super) fn session_destination_config(
    session: &AgentSession,
    issue_id: &str,
    issue_identifier: &str,
    issue_url: Option<&str>,
    team_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "session_id": session.id,
        "issue_id": issue_id,
        "identifier": issue_identifier,
        "team_id": team_id,
        "issue_url": issue_url,
        // The comment that spawned the session (mention). `None` for
        // assignment-initiated sessions. A session task is thread-scoped: it
        // hears a follow-up comment only when the comment replies to a comment
        // it authored or to this originating comment — see
        // `comments::recipients_for_comment`.
        "origin_comment_id": session.comment.as_ref().map(|c| c.id.as_str()),
    })
}

/// The `destination_config` of a Binding created from an issue entering a
/// trigger state. No `session_id`: there is no Linear session behind it.
pub(super) fn issue_destination_config(issue: &IssuePayload, team_id: &str) -> serde_json::Value {
    serde_json::json!({
        "issue_id": issue.id,
        "issue_identifier": issue.identifier,
        "identifier": issue.identifier,
        "team_id": team_id,
        "project_id": issue.resolved_project_id(),
        "issue_url": issue.url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two subject kinds are distinct strings, and both are persisted.
    ///
    /// Trivial to assert and expensive to get wrong: `subject_kind` is a stored
    /// discriminator, so making the two equal would merge two Binding
    /// populations that route replies through different Linear APIs.
    #[test]
    fn the_two_subject_kinds_are_distinct_and_pinned() {
        assert_eq!(SUBJECT_KIND_SESSION, "linear_agent_session");
        assert_eq!(SUBJECT_KIND_ISSUE, "linear_issue");
        assert_ne!(SUBJECT_KIND_SESSION, SUBJECT_KIND_ISSUE);
    }

    /// A session Binding carries `session_id`; an issue Binding does not.
    ///
    /// That difference is load-bearing: `send_reply_for_binding` and the
    /// comment heartbeat both branch on whether the key is present.
    #[test]
    fn only_a_session_binding_carries_a_session_id() {
        let session = AgentSession {
            id: "session-1".into(),
            issue: None,
            comment: None,
            creator: None,
            prompt_context: None,
        };
        let config = session_destination_config(
            &session,
            "issue-1",
            "ENG-7",
            Some("https://linear.app/acme/issue/ENG-7"),
            "team-1",
        );
        assert_eq!(config["session_id"], "session-1");
        assert_eq!(config["issue_id"], "issue-1");

        let issue = IssuePayload {
            id: "issue-1".into(),
            identifier: "ENG-7".into(),
            title: "Fix it".into(),
            url: None,
            team_id: Some("team-1".into()),
            team: None,
            project_id: None,
            project: None,
            updated_at: None,
            state: None,
        };
        let config = issue_destination_config(&issue, "team-1");
        assert!(
            config.get("session_id").is_none(),
            "an issue binding has no Linear session behind it"
        );
        assert_eq!(config["issue_id"], "issue-1");
    }
}
