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

use moatless_core::db::DbPool;

use moatless_loops::adapters::error::AdapterError;
use moatless_loops::adapters::models::{AdapterConnection, SubscriptionMatch, TaskBinding};
use moatless_loops::loops::dao::LoopDao;
use moatless_tasks::task::binding_dao::TaskBindingDao;

use super::SUBJECT_KIND_ISSUE;
use super::webhook_payloads::{AgentSession, IssuePayload};

pub(super) const ADAPTER: &str = "linear";
/// `task_bindings.subject_kind` for a Binding that names a Linear agent
/// session. Its `subject_key` is the session id.
pub(super) const SUBJECT_KIND_SESSION: &str = "linear_agent_session";

/// Which Bindings a lookup is asking for.
///
/// The two shapes differ only in which key they filter on, which is why they
/// are one query with a parameter rather than two statements: everything else
/// about them — the thirteen-column list, the adapter, the pending guard — has
/// to stay in step. It used to be spelled out twice, once inside a function
/// about closing tasks and once inside one about emitting activities (defect
/// D5), so a column rename had to find both.
pub(super) enum BindingScope<'a> {
    /// Every Binding of either subject kind pointing at this issue. Used by the
    /// close path, which must consider a task bound through a session and a
    /// task bound directly to the issue alike.
    OnIssue(&'a str),
    /// Every agent-session Binding held by this task. Used by the activity
    /// emitters, which have a task id and need the sessions to emit into.
    SessionsOfTask(&'a str),
}

/// Bindings matching `scope`, excluding any still marked pending.
///
/// A pending Binding is one whose provisioning has not finished. Acting on one
/// — closing its task, emitting into its session — would be acting on a task
/// that does not exist yet, so the guard is part of the query rather than left
/// to each caller to remember.
///
/// Note this deliberately does *not* filter on `active`: the close path needs
/// to see a Binding in order to decide about it, and the emitters' callers
/// filter for themselves. Callers that want only live Bindings say so.
pub(super) async fn bindings_in_scope(
    db: &DbPool,
    scope: BindingScope<'_>,
) -> Result<Vec<TaskBinding>, AdapterError> {
    let (kinds, issue_id, task_id) = match scope {
        BindingScope::OnIssue(issue_id) => (
            vec![
                SUBJECT_KIND_SESSION.to_string(),
                SUBJECT_KIND_ISSUE.to_string(),
            ],
            Some(issue_id),
            None,
        ),
        BindingScope::SessionsOfTask(task_id) => {
            (vec![SUBJECT_KIND_SESSION.to_string()], None, Some(task_id))
        }
    };

    // One literal statement, selected by its bound parameters rather than by
    // string assembly: exactly one of `$3`/`$4` is ever non-null, and the enum
    // is what guarantees it.
    sqlx::query_as::<_, TaskBinding>(
        "SELECT id, task_id, adapter_kind, subject_kind, subject_key, connection_id, \
                loop_id, destination_config, metadata, last_external_event_id, \
                origin, created_at, last_event_at \
         FROM task_bindings \
         WHERE adapter_kind = $1 \
           AND subject_kind = ANY($2) \
           AND ($3::text IS NULL OR destination_config->>'issue_id' = $3) \
           AND ($4::text IS NULL OR task_id = $4) \
           AND COALESCE(destination_config->>'pending', 'false') <> 'true'",
    )
    .bind(ADAPTER)
    .bind(&kinds)
    .bind(issue_id)
    .bind(task_id)
    .fetch_all(db)
    .await
    .map_err(AdapterError::from)
}

/// Every active Binding of either subject kind listening to `issue_id`, one per
/// task, scoped to `connection`.
///
/// Session Bindings are queried first and win the de-duplication, because a
/// task holding both kinds on one issue should be reached through the one that
/// has a Linear panel to heartbeat into.
pub(super) async fn comment_recipients(
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

    let mut seen_tasks = std::collections::HashSet::new();
    Ok(rows
        .into_iter()
        // Skip bindings whose task has closed — a closed task is terminal and
        // must not be resurrected by a new comment.
        .filter(|b| b.connection_id == connection.id && b.active)
        .filter(|b| seen_tasks.insert(b.task_id.clone()))
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
    loops: &LoopDao,
    binding: &TaskBinding,
) -> Result<Option<SubscriptionMatch>, AdapterError> {
    match binding.loop_id {
        Some(ref loop_id) => Ok(loops.find_subscription_by_id(loop_id).await?),
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
