//! One conversation's detail: its checkpoints, plans and subtasks.

use std::collections::HashMap;

use serde_json::{Value, json};
use tracing::warn;

use super::origin::OriginContext;
use super::scalar::created_at;
use super::scalar::instant;
use super::scalar::non_empty;
use super::scalar::nullable_instant;
use super::shell::conversation_row;
use super::transcript::activity_anchor;
use super::transcript::message_instant;
use super::transcript::rendered_transcript;
use super::turn::turn_id;
use super::turn::turn_is_open;
use crate::AgentModelDefaults;
use crate::dao::Conversation;
use crate::transcript_dao::{Transcript, TranscriptMessage};
use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::message::MessageType;
use moatless_types::task::{TaskRelationType, TurnStatus};

/// One conversation, in the shape the thread view carries: the shared row plus
/// the transcript.
///
/// `checkpoints` is the completed turns — see [`checkpoints`].
///
/// `proposedPlans` is read back out of the transcript — see [`proposed_plans`].
///
/// The four shell-only fields are absent rather than false: they are not part
/// of the contract's full thread, and the client reads a conversation's pending
/// state from its activities here.
pub(crate) fn thread_detail(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    transcript: &Transcript,
    origins: &OriginContext<'_>,
) -> Value {
    let mut row = conversation_row(conversation, project_id, defaults);
    let rendered = rendered_transcript(conversation, transcript, origins);
    // Required present, and a conversation that no longer exists is not
    // projected at all, so null is the only value this can carry. Deleting is
    // not archiving: `archivedAt` above is the close time.
    row.insert("deletedAt".into(), Value::Null);
    row.insert("messages".into(), Value::Array(rendered.messages));
    row.insert("activities".into(), Value::Array(rendered.activities));
    row.insert(
        "proposedPlans".into(),
        Value::Array(proposed_plans(conversation, transcript)),
    );
    row.insert(
        "checkpoints".into(),
        Value::Array(checkpoints(conversation, transcript)),
    );
    Value::Object(row)
}

/// The turns a person may rewind this conversation to, oldest first.
///
/// A T3 checkpoint is a per-turn worktree snapshot and Moatless takes none, so
/// `files` is empty and `checkpointRef` names the turn rather than a tree. Every
/// rewind decision the client makes reads `checkpointTurnCount`, and
/// `commands::dispatch` refuses the file-restoring half.
///
/// `status` is `ready` and never `missing`, even carrying no files:
/// `resolveProactiveTurnDiffAction` defers on `missing` and its caller keeps the
/// turn eligible, so a completed turn would be re-examined for as long as the
/// thread stays open.
///
/// `checkpointTurnCount` is `task_turns.turn_number` itself and not a position
/// in this list, because the number the client sends back is the cut
/// `TaskRewindService::rewind` takes verbatim. The set is [`Transcript::turns`]
/// and not the Messages' turn numbers, so it is the set that cut is decided
/// against; a turn with no Message to read a label off reports a null
/// `assistantMessageId` and the conversation's own instant.
///
/// Empty while a turn is in flight: `rewind` refuses every cut while one is,
/// including a cut below it, unlike a fork.
pub(super) fn checkpoints(conversation: &Conversation, transcript: &Transcript) -> Vec<Value> {
    if turn_is_open(conversation) {
        return Vec::new();
    }

    // One pass rather than a scan per turn: each turn's last assistant bubble,
    // which is the one the client scans forward to when it labels the revert
    // action, and its last Message of any kind to fall back on.
    let mut labels: HashMap<i32, (Option<&TranscriptMessage>, &TranscriptMessage)> = HashMap::new();
    for message in &transcript.messages {
        let Some(number) = message.turn_number else {
            continue;
        };
        let entry = labels.entry(number).or_insert((None, message));
        entry.1 = message;
        if message.message_type == MessageType::Assistant
            && message
                .message_text
                .as_deref()
                .is_some_and(|text| !text.is_empty())
        {
            entry.0 = Some(message);
        }
    }

    transcript
        .turns
        .iter()
        .copied()
        .map(|number| {
            let labelled = labels.get(&number);
            let closing = labelled.and_then(|(closing, _)| *closing);
            let completed_at = closing
                .or_else(|| labelled.map(|(_, last)| *last))
                .map_or_else(
                    || instant(created_at(conversation)),
                    |message| instant(message_instant(conversation, message)),
                );
            json!({
                "turnId": turn_id(&conversation.task_id, number),
                "checkpointTurnCount": number,
                "checkpointRef": turn_id(&conversation.task_id, number),
                "status": "ready",
                "files": Value::Array(Vec::new()),
                "assistantMessageId": closing.map_or(Value::Null, |message| json!(message.uuid)),
                "completedAt": completed_at
            })
        })
        .collect()
}

/// The plans the agent proposed, answered or not.
///
/// A Moatless proposed plan is an `ExitPlanMode` call: the run parks on it, and
/// an answer releases it. The list is read off the transcript rather than
/// stored, one entry per call, identified by the call's own activity id.
///
/// An answered call keeps its entry and carries `implementedAt`, which the
/// client reads to stop offering the plan. A rejected call carries it too: the
/// field says the plan was answered, not that it was approved.
fn proposed_plans(conversation: &Conversation, transcript: &Transcript) -> Vec<Value> {
    transcript
        .activities
        .iter()
        .filter(|activity| {
            activity.kind == ActivityKind::ToolCall
                && activity.summary == moatless_types::activity::PLAN_TOOL
        })
        .filter_map(|activity| {
            let plan = activity
                .tool_input()
                .get("plan")
                .and_then(Value::as_str)
                .filter(|plan| !plan.trim().is_empty())?;
            let (turn, at) = activity_anchor(conversation, transcript, activity);
            let implemented_at = answered_at(transcript, activity);
            Some(json!({
                "id": activity.activity_id,
                "turnId": turn,
                "planMarkdown": plan,
                "implementedAt": implemented_at,
                "implementationThreadId": Value::Null,
                "createdAt": at,
                "updatedAt": implemented_at.as_deref().unwrap_or(at.as_str())
            }))
        })
        .collect()
}

/// When the answer to a parked interactive call landed, or `None` while it is
/// still parked.
///
/// The join is `requestId`: the resolved row leaves its own `tool_use_id` column
/// empty and carries the call's id in its payload instead
/// (`mapping::user_input_resolved`). A settled call whose resolved row the
/// window does not reach reports the call's own instant, so that no settled
/// call reads as still parked.
fn answered_at(transcript: &Transcript, call: &Activity) -> Option<String> {
    if !call.is_settled() {
        return None;
    }
    // A row with no id to join on matches nothing: comparing two `None`s here
    // would take the first resolved row that carries no `requestId` either.
    let resolved = call.tool_use_id.as_deref().and_then(|tool_use_id| {
        transcript.activities.iter().find(|activity| {
            activity.kind == ActivityKind::UserInputResolved
                && activity.payload.get("requestId").and_then(Value::as_str) == Some(tool_use_id)
        })
    });
    Some(instant(resolved.unwrap_or(call).created_at))
}

/// One child Task, in the shape `Subtask` has on the wire
/// (`packages/contracts/src/subtasks.ts`).
///
/// Deliberately not a thread row: a subtask is rendered as a fleet member
/// beside the subagents of the Task it belongs to, so it carries what that row
/// shows and a `threadId` to open it by. The client that follows the link gets
/// the model, worktree and session fields from the thread itself.
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
        "agentType": conversation.agent_type.as_str(),
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
/// Closing is Moatless's way of saying the work is done, so it outranks the
/// newest turn: a closed Task whose last turn errored is finished, not failing.
/// A Task that has never run is `pending`, and one whose last turn completed
/// while the Task is still open is `idle`.
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

/// The head of a transcript that did not fit, said out loud.
///
/// A capped read that simply started in the middle would look like a
/// conversation that began there. This is the one message in a transcript that
/// no row in the database backs, which is why it says so.
pub(super) fn elision_notice(conversation: &Conversation, transcript: &Transcript) -> Value {
    let at = instant(transcript.messages.first().map_or_else(
        || created_at(conversation),
        |first| message_instant(conversation, first),
    ));
    json!({
        "id": format!("{}:elided", conversation.task_id),
        "role": "system",
        "text": format!(
            "{} earlier messages are not shown in this view.",
            transcript.omitted
        ),
        "turnId": Value::Null,
        "streaming": false,
        "createdAt": at,
        "updatedAt": at
    })
}
