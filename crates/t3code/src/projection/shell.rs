//! The shell: the listing T3 opens with, and one conversation's row in it.

use std::collections::HashSet;

use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use tracing::warn;

use super::model::model_selection;
use super::project::project;
use super::pull_request::legacy_linked_pull_request;
use super::pull_request::thread_pull_requests;
use super::scalar::created_at;
use super::scalar::instant;
use super::scalar::non_empty;
use super::scalar::nullable_instant;
use super::session::RUNTIME_MODE;
use super::session::session;
use super::session::worktree_path;
use super::turn::latest_turn;
use crate::AgentModelDefaults;
use crate::dao::Conversation;
use crate::workspace::UiRpcListedWorkspace;

/// The shell listing: the viewer's projects, and the conversations in them.
///
/// A conversation whose project is absent from `workspaces` is left out here.
/// The client skips one whose project it cannot find with a bare `continue`,
/// so the row would vanish with nothing logged; dropping it here leaves a
/// warning behind.
///
/// `sequence` is the listing position the snapshot was read at, and it has to
/// be read *before* the rows. The client discards every later event at or below
/// it, so a position taken afterwards would discard the event for a change the
/// rows already reflect. Taken beforehand the worst case is one change
/// delivered twice.
pub(crate) fn shell_snapshot(
    workspaces: &[UiRpcListedWorkspace],
    conversations: &[Conversation],
    defaults: &AgentModelDefaults,
    sequence: i64,
) -> Value {
    json!({
        "snapshotSequence": sequence,
        "projects": workspaces.iter().map(project).collect::<Vec<_>>(),
        "threads": listed_threads(workspaces, conversations, defaults),
        "updatedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    })
}

/// The rows of a listing, dropping each conversation whose project `workspaces`
/// does not carry — see [`shell_snapshot`] for why that drop is here.
pub(crate) fn listed_threads(
    workspaces: &[UiRpcListedWorkspace],
    conversations: &[Conversation],
    defaults: &AgentModelDefaults,
) -> Vec<Value> {
    let projects: HashSet<&str> = workspaces
        .iter()
        .map(|listed| listed.workspace.id.as_str())
        .collect();
    conversations
        .iter()
        .filter_map(|conversation| {
            let project_id = conversation
                .workspace_id
                .as_deref()
                .filter(|id| projects.contains(id));
            let Some(project_id) = project_id else {
                warn!(
                    task_id = %conversation.task_id,
                    workspace_id = ?conversation.workspace_id,
                    "Conversation has no listable workspace; leaving it out of the T3 shell listing"
                );
                return None;
            };
            Some(thread(conversation, project_id, defaults))
        })
        .collect()
}

/// One conversation's row, re-read and sent whole, at the position that carried
/// it.
///
/// Never a per-change difference: the client applies this as an upsert keyed on
/// the conversation's id and moves its position to `sequence`, so what it needs
/// is the current row rather than an account of what moved. That is also what
/// makes coalescing safe — a burst collapses into one of these, and the burst's
/// last position is the one it carries.
pub(crate) fn thread_upserted(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    sequence: i64,
) -> Value {
    json!({
        "kind": "thread-upserted",
        "sequence": sequence,
        "thread": thread(conversation, project_id, defaults)
    })
}

/// One conversation leaving a viewer's listing.
///
/// Built only for a conversation the viewer was previously sent — see
/// `ShellFeed`. The id is the whole payload, so emitting one whenever a re-read
/// came back empty would tell every connected client the id of every
/// conversation that changed anywhere.
pub(crate) fn thread_removed(task_id: &str, sequence: i64) -> Value {
    json!({
        "kind": "thread-removed",
        "sequence": sequence,
        "threadId": task_id
    })
}

/// One conversation's listing row, asked for by id rather than arrived at
/// through a listing.
///
/// The row is the listing's, to the field: what a client holds for a
/// conversation must not depend on which read delivered it, or the same
/// conversation would render two ways depending on how it was reached.
pub(crate) fn thread_shell(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
) -> Value {
    thread(conversation, project_id, defaults)
}

/// One conversation, in the shape the shell listing carries.
///
/// This is the contract's *shell* thread, not its full one: no messages, no
/// activities, no checkpoints, no proposed plans, and no `deletedAt`. See
/// [`thread_detail`] for the other member of the pair.
pub(super) fn thread(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
) -> Value {
    let mut row = conversation_row(conversation, project_id, defaults);
    // T3 counts only messages a person wrote, which is what keeps the agent's
    // own output out of the sidebar's ordering entirely.
    row.insert(
        "latestUserMessageAt".into(),
        nullable_instant(conversation.latest_user_message_at),
    );
    // The contract requires a definite answer and supplies no empty value.
    // Moatless has no approval queue, so the answer is always no.
    row.insert("hasPendingApprovals".into(), Value::Bool(false));
    row.insert(
        "hasPendingUserInput".into(),
        Value::Bool(conversation.pending_question),
    );
    row.insert(
        "hasActionableProposedPlan".into(),
        Value::Bool(conversation.pending_plan),
    );
    // Null rather than absent, because the client reads the key to clear its
    // previous answer. Which rows carry a name is the listing's decision; see
    // `Conversation::owner_name`.
    row.insert(
        "ownerName".into(),
        conversation
            .owner_name
            .as_deref()
            .and_then(non_empty)
            .map_or(Value::Null, Value::String),
    );
    // A `moat cmd` command outlives the turn that started it, and the agent
    // reports idle the moment that turn ends. `working` rather than
    // `monitoring`: the contract reserves that for watch loops with no other
    // live work, and a build is work.
    row.insert(
        "backgroundLiveness".into(),
        if conversation.running_commands > 0 {
            Value::String("working".into())
        } else {
            Value::Null
        },
    );
    Value::Object(row)
}

/// The fields both shapes of a conversation carry, with the same values.
///
/// `interactionMode` is deliberately absent rather than null: it has no
/// separate Moatless state. `archivedAt` is the Task's close time, because
/// archiving a conversation is closing it. Settlement and snoozing are follower
/// state and are emitted here.
pub(super) fn conversation_row(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
) -> Map<String, Value> {
    let created = created_at(conversation);
    let pull_requests = thread_pull_requests(conversation);
    let Value::Object(row) = json!({
        "id": conversation.task_id,
        "projectId": project_id,
        // `tasks.name` is nullable and has no non-empty constraint, and the
        // contract refuses a blank title, so an unnamed conversation shows the
        // contract's "New thread" placeholder until it is auto-titled.
        "title": conversation
            .name
            .as_deref()
            .and_then(non_empty)
            .unwrap_or_else(|| "New thread".to_string()),
        "modelSelection": model_selection(conversation, defaults),
        "runtimeMode": RUNTIME_MODE,
        "branch": conversation
            .branch
            .as_deref()
            .and_then(non_empty)
            .map_or(Value::Null, Value::String),
        "worktreePath": worktree_path(conversation),
        // `linkedPullRequest` is the primary of `pullRequests`. Both are
        // emitted: a client from before `threadPullRequests` reads the single
        // reference, and the capability is what tells a newer one to prefer the
        // list.
        "linkedPullRequest": legacy_linked_pull_request(pull_requests.first(), project_id),
        "pullRequests": Value::Array(pull_requests),
        "latestTurn": latest_turn(conversation),
        "createdAt": instant(created),
        "updatedAt": instant(conversation.updated_at.unwrap_or(created)),
        "archivedAt": nullable_instant(conversation.closed_at),
        "session": session(conversation),
        "settledOverride": conversation
            .settled_override
            .as_deref()
            .map_or(Value::Null, |value| Value::String(value.to_string())),
        "settledAt": nullable_instant(conversation.settled_at),
        "snoozedUntil": nullable_instant(conversation.snoozed_until),
        "snoozedAt": nullable_instant(conversation.snoozed_at),
        "visibility": conversation.visibility.as_str()
    }) else {
        unreachable!("a json! object literal is an object")
    };
    row
}
