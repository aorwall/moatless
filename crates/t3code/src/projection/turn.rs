//! The Turn a conversation is on, and what the client may send while it runs.

use serde_json::{Value, json};
use tracing::warn;

use super::scalar::nullable_instant;
use super::scalar::timestamp;
use crate::dao::Conversation;
use crate::transcript_dao::TranscriptMessage;
use moatless_types::running::RunningInputs;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TurnStatus;

/// The Turn this Message belongs to, in T3's identity for one.
pub(super) fn turn_reference(conversation: &Conversation, message: &TranscriptMessage) -> Value {
    message.turn_number.map_or(Value::Null, |number| {
        Value::String(turn_id(&conversation.task_id, number))
    })
}

/// The conversation's newest turn, or null when it has never run.
///
/// A Moatless turn comes into existence already running, so `requestedAt` and
/// `startedAt` both report `turn_started_at`; `assistantMessageId` is null
/// since the shell listing carries no messages.
pub(super) fn latest_turn(conversation: &Conversation) -> Value {
    let (Some(turn_number), Some(status)) = (
        conversation.turn_number,
        conversation.turn_status.as_deref(),
    ) else {
        return Value::Null;
    };
    let Some(state) = turn_state(status) else {
        warn!(
            task_id = %conversation.task_id,
            status,
            "Turn status has no T3 counterpart; reporting no latest turn"
        );
        return Value::Null;
    };
    json!({
        "turnId": turn_id(&conversation.task_id, turn_number),
        "state": state,
        "requestedAt": timestamp(conversation.turn_started_at),
        "startedAt": timestamp(conversation.turn_started_at),
        "completedAt": nullable_instant(conversation.turn_ended_at),
        "assistantMessageId": Value::Null
    })
}

/// Moatless turn state in T3's four-value vocabulary.
///
/// Two pairings are not the name match: Moatless's *stopped* (a person cutting
/// it short) is T3's *interrupted*, while Moatless's own *interrupted* (Sandbox
/// went away mid-turn) stays **open**/running, since it resumes on reschedule.
/// `None` for a status this build does not know.
pub(super) fn turn_state(status: &str) -> Option<&'static str> {
    Some(match TurnStatus::parse(status)? {
        TurnStatus::Active | TurnStatus::Interrupted => "running",
        TurnStatus::Completed => "completed",
        TurnStatus::Error => "error",
        TurnStatus::Stopped => "interrupted",
    })
}

/// Whether the newest turn is still open. Both Moatless states that mean *not
/// finished* count, which is the same grouping the rest of this backend uses
/// when it asks which turns are still running.
pub(super) fn turn_is_open(conversation: &Conversation) -> bool {
    conversation
        .turn_status
        .as_deref()
        .and_then(TurnStatus::parse)
        .is_some_and(TurnStatus::is_open)
}

/// The signals the running rule reads, as this projection can see them.
///
/// A listing reads rows, never the runtime: `agent` is `None` since the
/// heartbeat reaches these rows through the reaper, and `sandbox_up` is read
/// from `sandbox_started_at` for the same reason.
pub(super) fn running_inputs(
    conversation: &Conversation,
    desired_state: SandboxDesiredState,
) -> RunningInputs {
    RunningInputs {
        desired_state: Some(desired_state),
        stop_reason: conversation.sandbox_stop_reason(),
        sandbox_up: Some(conversation.sandbox_started_at.is_some()),
        turn_status: conversation
            .turn_status
            .as_deref()
            .and_then(TurnStatus::parse),
        turn_ended_at: conversation.turn_ended_at,
        latest_user_message_at: conversation.latest_queued_message_at,
        agent: None,
    }
}

/// A turn's identity on the wire.
///
/// Moatless identifies a turn by (task, turn number) with no single-column id,
/// so the pair is joined into one — stable and unique across the deployment, as
/// the contract's identity requires.
pub(super) fn turn_id(task_id: &str, turn_number: i32) -> String {
    format!("{task_id}:{turn_number}")
}
