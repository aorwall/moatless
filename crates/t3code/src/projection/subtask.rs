//! One child Task as the subtask fleet lists it.

use serde_json::{Value, json};
use tracing::warn;

use super::scalar::created_at;
use super::scalar::instant;
use super::scalar::non_empty;
use super::scalar::nullable_instant;
use crate::dao::Conversation;
use moatless_types::task::{TaskRelationType, TurnStatus};

/// One child Task, in the shape `Subtask` has on the wire
/// (`packages/contracts/src/subtasks.ts`).
///
/// Deliberately not a thread row: rendered as a fleet member beside the Task's
/// subagents, carrying what that row shows plus a `threadId` — the client
/// follows the link for the model, worktree and session fields.
pub(crate) fn subtask(conversation: &Conversation, relation: TaskRelationType) -> Value {
    json!({
        "threadId": conversation.task_id,
        "title": conversation
            .name
            .as_deref()
            .and_then(non_empty)
            .unwrap_or_else(|| "New thread".to_string()),
        "relation": match relation {
            TaskRelationType::ForkedFrom => "forkedFrom",
            // `related_to` is not a parent edge and never reaches this
            // projection; the listing filters it out before projecting.
            _ => "createdBy",
        },
        "status": subtask_status(conversation),
        // Both are the agent parked on a person's answer, and a subtask's row
        // says so rather than reporting the run as still working.
        "awaitingInput": conversation.pending_question || conversation.pending_plan,
        "agentProfile": conversation.agent_profile.as_str(),
        "branch": conversation
            .branch
            .as_deref()
            .and_then(non_empty)
            .map_or(Value::Null, Value::String),
        // The newest turn's number is how many turns have run, which is what a
        // fleet row shows as the child's depth of work.
        "turnCount": conversation.turn_number.unwrap_or(0).max(0),
        "createdAt": instant(created_at(conversation)),
        "updatedAt": instant(conversation.updated_at.unwrap_or_else(|| created_at(conversation))),
        "archivedAt": nullable_instant(conversation.closed_at),
    })
}

/// A child Task's state in the agents panel's own status vocabulary, so a
/// subtask row and a subagent row read the same.
///
/// Closing outranks the newest turn: a closed Task whose last turn errored
/// reads as finished, not failing. Never run is `pending`; completed while
/// still open is `idle`.
fn subtask_status(conversation: &Conversation) -> &'static str {
    if conversation.closed_at.is_some() {
        return "completed";
    }
    let Some(status) = conversation.turn_status.as_deref() else {
        return "pending";
    };
    let Some(parsed) = TurnStatus::parse(status) else {
        warn!(
            task_id = %conversation.task_id,
            status,
            "Turn status has no T3 counterpart; reporting the subtask as pending"
        );
        return "pending";
    };
    match parsed {
        TurnStatus::Active | TurnStatus::Interrupted => "running",
        TurnStatus::Completed => "idle",
        TurnStatus::Error => "failed",
        TurnStatus::Stopped => "interrupted",
    }
}
