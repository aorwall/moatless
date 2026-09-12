//! The Turn a conversation is on, and what the client may send while it runs.

use serde_json::{Value, json};
use tracing::warn;

use super::scalar::nullable_instant;
use super::scalar::timestamp;
use crate::dao::{Conversation, TranscriptMessage};
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
/// A Moatless turn comes into existence already running, so there is no
/// requested-but-not-started instant distinct from the start: both report the
/// turn's `started_at`.
///
/// `assistantMessageId` is null because the shell listing carries no messages.
/// Choosing the identity a message reports belongs with the projection that
/// emits messages, and inventing one here that a later phase then has to match
/// would be the more expensive mistake.
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
/// Five states map onto four. Two of the pairings are not the name match:
/// *stopped* is a person cutting a turn short, which is what T3 calls
/// *interrupted*; and Moatless's own *interrupted* means the Sandbox went away
/// mid-turn, which leaves the turn **open** — it resumes on reschedule, and the
/// reaper reconciles it into a terminal state if it never does. Reporting that
/// as T3's *interrupted* would show a turn that is about to carry on as a turn
/// that ended, and the sidebar would quietly stop showing live work.
///
/// `None` for a status this build does not know — a row written by a newer
/// backend, which has no honest counterpart.
pub(super) fn turn_state(status: &str) -> Option<&'static str> {
    Some(match TurnStatus::parse(status)? {
        TurnStatus::Active | TurnStatus::Interrupted => "running",
        TurnStatus::Completed => "completed",
        TurnStatus::Error => "error",
        TurnStatus::Stopped => "interrupted",
    })
}

/// Whether the newest turn is still open. Both Moatless states that mean
/// *not finished* count, which is the same grouping the rest of this backend
/// uses when it asks which turns are still running.
pub(super) fn turn_is_open(conversation: &Conversation) -> bool {
    conversation
        .turn_status
        .as_deref()
        .and_then(TurnStatus::parse)
        .is_some_and(TurnStatus::is_open)
}

/// The signals the running rule reads, as this projection can see them.
///
/// A listing reads rows and never the runtime, so `agent` is `None` — the
/// heartbeat reaches these rows through the reaper rather than through a
/// per-conversation call this poll could not afford. `sandbox_up` is read from
/// `sandbox_started_at` for the same reason: no observed pod state is at hand.
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
        latest_user_message_at: conversation.latest_user_message_at,
        agent: None,
    }
}

/// A turn's identity on the wire.
///
/// Moatless identifies a turn by (task, turn number) and has no single-column
/// id, so the pair is joined into one. Stable across reads and unique across
/// the deployment, which is what the contract's identity needs.
pub(super) fn turn_id(task_id: &str, turn_number: i32) -> String {
    format!("{task_id}:{turn_number}")
}
