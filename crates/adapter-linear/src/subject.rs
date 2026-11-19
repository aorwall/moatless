//! What a Linear Binding points at, and how to find one.
//!
//! Two subject kinds. A `linear_agent_session` Binding names a Linear agent
//! session with a live panel, kept alive by an activity, that replies land
//! in. A `linear_issue` Binding (CLI, operator, or trigger-state) names an
//! issue with no session, panel, or activity calls — only comments.
//!
//! Both carry `issue_id` in `destination_config`; both destination-config
//! literals live here, not at their two provisioning sites, since a
//! differently-spelled key would write a Binding nothing could find.

use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::bindings::operations::Bindings;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::TaskBinding;
use moatless_types::loops::SubscriptionMatch;

use super::webhook_payloads::{AgentSession, IssuePayload};

pub(super) use super::delivery::ADAPTER;
/// `task_bindings.subject_kind` for a Binding that names a Linear agent
/// session. Its `subject_key` is the session id.
pub(super) const SUBJECT_KIND_SESSION: &str = "linear_agent_session";

/// Which Bindings a lookup is asking for.
///
/// The two shapes differ only in which key they filter on, so this is one
/// lookup with a parameter rather than two — everything else (adapter, subject
/// kinds, pending guard) has to stay in step.
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
/// Skips a Binding still pending in `destination_config` (its Linear side
/// doesn't exist yet). Deliberately not filtered on `active` — the close
/// path needs to see a Binding to decide about it, so callers wanting only
/// live ones filter themselves.
pub(super) async fn bindings_in_scope(
    task_bindings: &Bindings,
    scope: BindingScope<'_>,
) -> Result<Vec<TaskBinding>, AdapterError> {
    match scope {
        BindingScope::OnIssue(issue_id) => {
            let mut rows = Vec::new();
            for subject_kind in [SUBJECT_KIND_SESSION, SUBJECT_KIND_ISSUE] {
                rows.extend(
                    task_bindings
                        .by_destination_field(ADAPTER, subject_kind, "issue_id", issue_id)
                        .await?,
                );
            }
            Ok(rows)
        }
        BindingScope::SessionsOfTask(task_id) => Ok(task_bindings
            .for_task(task_id)
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

/// Every active Binding of either subject kind listening to `issue_id`,
/// scoped to `connection`, session Bindings first.
///
/// Not de-duplicated per task: which Binding hears a comment depends on the
/// comment (see `comments::recipients_for_comment`); session-first order lets
/// the recipient step keep the session Binding when it matched.
pub(super) async fn active_comment_bindings(
    task_bindings: &Bindings,
    connection: &AdapterConnection,
    issue_id: &str,
) -> Result<Vec<TaskBinding>, AdapterError> {
    let mut rows = Vec::new();
    for subject_kind in [SUBJECT_KIND_SESSION, SUBJECT_KIND_ISSUE] {
        rows.extend(
            task_bindings
                .by_destination_field(ADAPTER, subject_kind, "issue_id", issue_id)
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

/// Bindings of `subject_kind`, active or not, pointing at `issue_id`, scoped
/// to `connection`.
pub(super) async fn bindings_on_issue(
    task_bindings: &Bindings,
    connection: &AdapterConnection,
    subject_kind: &str,
    issue_id: &str,
) -> Result<Vec<TaskBinding>, AdapterError> {
    let rows = task_bindings
        .by_destination_field(ADAPTER, subject_kind, "issue_id", issue_id)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|b| b.connection_id == connection.id)
        .collect())
}

/// The Subscription a Binding was created by, if it still exists.
///
/// `None` means "no Subscription context", not "stop" — a Binding whose
/// Subscription was deleted keeps receiving events; only new trigger-created
/// Task behavior is gated on it.
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
/// `session_id` is what makes it a session Binding — the reply path and
/// heartbeat both read it, and its absence is what distinguishes the two kinds
/// at a call site that only has the JSON.
pub(super) fn session_destination_config(
    session: &AgentSession,
    issue_id: &str,
    issue_identifier: &str,
    issue_url: Option<&str>,
    team_id: &str,
) -> serde_json::Value {
    let mut config = serde_json::json!({
        "session_id": session.id,
        "issue_id": issue_id,
        "identifier": issue_identifier,
        "team_id": team_id,
        "issue_url": issue_url,
    });
    // The mention that spawned the session; null for an assignment-initiated
    // session, which starts inside no thread.
    config[ORIGIN_COMMENT_ID] = serde_json::json!(session.comment.as_ref().map(|c| c.id.as_str()));
    config
}

/// `destination_config.origin_comment_id`: the comment a Binding is scoped to.
///
/// A session Binding carries the mention that spawned it; an issue Binding, the
/// comment its own task posted. `comments::hears_comment` decides what a scoped
/// Binding hears.
pub(super) const ORIGIN_COMMENT_ID: &str = "origin_comment_id";

/// The comment `binding` is scoped to, or `None` when it hears its whole issue.
pub(super) fn origin_comment_id(binding: &TaskBinding) -> Option<&str> {
    binding
        .destination_config
        .get(ORIGIN_COMMENT_ID)
        .and_then(serde_json::Value::as_str)
}

/// Whether this issue Binding hears only replies to what its Task wrote,
/// rather than connecting the Task to the whole issue: one scoped to a comment
/// the Task posted, or one whose role `hears_only_replies`, as a T3 link's is.
///
/// `close.rs` leaves it out of the sibling count for that reason: commenting on
/// a ticket is not taking on a second connected subject.
pub(super) fn hears_only_replies_on_issue(binding: &TaskBinding) -> bool {
    binding.subject_kind == SUBJECT_KIND_ISSUE
        && (origin_comment_id(binding).is_some()
            || moatless_adapter_model::binding::hears_only_replies(binding))
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

/// `task_bindings.subject_kind` for bindings that point to a Linear issue
/// (CLI-created or operator-initiated).
///
/// Distinct from `linear_agent_session`, which routes replies through
/// `agentActivityCreate`; `linear_issue` routes through `commentCreate`.
pub const SUBJECT_KIND_ISSUE: &str = "linear_issue";

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
    /// Load-bearing: `send_reply_for_binding` and the comment heartbeat both
    /// branch on whether the key is present.
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
            label_ids: Vec::new(),
            labels: Vec::new(),
        };
        let config = issue_destination_config(&issue, "team-1");
        assert!(
            config.get("session_id").is_none(),
            "an issue binding has no Linear session behind it"
        );
        assert_eq!(config["issue_id"], "issue-1");
    }

    fn binding(subject_kind: &str, destination_config: serde_json::Value) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: "ts_1".into(),
            task_id: "task-1".into(),
            adapter_kind: moatless_types::adapter::AdapterKind::Linear,
            subject_kind: subject_kind.to_string(),
            subject_key: "issue-1".into(),
            connection_id: "pc_1".into(),
            loop_id: None,
            destination_config,
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "responder".into(),
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

    /// Only a `linear_issue` Binding scoped to a comment or narrowed by its
    /// role hears only replies.
    ///
    /// `close.rs` drops these from its sibling count, so widening this to a
    /// session Binding — which carries `origin_comment_id` whenever a mention
    /// spawned it — would stop that task from holding the issue open.
    #[test]
    fn only_an_issue_binding_that_hears_replies_is_excluded_from_the_sibling_count() {
        let scoped = serde_json::json!({ "issue_id": "issue-1", ORIGIN_COMMENT_ID: "comment-1" });
        assert!(hears_only_replies_on_issue(&binding(
            SUBJECT_KIND_ISSUE,
            scoped.clone()
        )));
        assert!(!hears_only_replies_on_issue(&binding(
            SUBJECT_KIND_SESSION,
            scoped
        )));

        let whole_issue = serde_json::json!({ "issue_id": "issue-1" });
        assert!(!hears_only_replies_on_issue(&binding(
            SUBJECT_KIND_ISSUE,
            whole_issue
        )));
        let mut linked = binding(
            SUBJECT_KIND_ISSUE,
            serde_json::json!({ "issue_id": "issue-1" }),
        );
        linked.metadata = serde_json::json!({
            moatless_adapter_model::binding::BIND_REASON_KEY: "contributor",
        });
        assert!(
            hears_only_replies_on_issue(&linked),
            "a T3 link is narrowed by its role, with no comment to scope it"
        );
    }
}
