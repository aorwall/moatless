//! The change events a subscription sends between snapshots.

use serde_json::{Value, json};

use super::origin::OriginContext;
use super::scalar::created_at;
use super::scalar::instant;
use super::session::session;
use super::transcript::activity_anchor;
use super::transcript::activity_row;
use super::transcript::activity_slot;
use super::transcript::message_bubble;
use super::transcript::message_instant;
use super::turn::turn_reference;
use crate::dao::Conversation;
use crate::transcript_dao::Transcript;
use moatless_core::change_feed::conversation::ConversationChange;
use moatless_types::activity::Activity;

/// Turns one recorded change into stream items, in the order a client applies
/// them.
///
/// Sequences are capped at [`EVENTS_PER_CHANGE`] per change; a Message outside
/// `window` says nothing.
pub(crate) fn change_events(
    conversation: &Conversation,
    change: &ConversationChange,
    window: &Transcript,
    origins: &OriginContext<'_>,
) -> Vec<Value> {
    if let Some(turn_count) = change.reverted_to_turn {
        return revert_event(conversation, change, turn_count)
            .into_iter()
            .collect();
    }
    if let Some(activity_id) = change.activity_id.as_deref() {
        return activity_change_events(conversation, change, window, activity_id);
    }
    let Some(uuid) = change.message_uuid.as_deref() else {
        return session_set(conversation, change).into_iter().collect();
    };
    let message = window.messages.iter().find(|message| message.uuid == uuid);
    let activities: Vec<&Activity> = window
        .activities
        .iter()
        .filter(|activity| activity.message_uuid.as_deref() == Some(uuid))
        .collect();
    if message.is_none() && activities.is_empty() {
        return Vec::new();
    }

    // The envelope's instant is the changed row's own. A change naming no
    // Message is dated by the first activity under it.
    let at = message.map_or_else(
        || {
            activities.first().map_or_else(
                || instant(created_at(conversation)),
                |activity| instant(activity.created_at),
            )
        },
        |message| instant(message_instant(conversation, message)),
    );

    // Each item numbered by the slot it owns rather than by where it lands in
    // this list: the bubble holds slot 0 whether or not it is emitted, and each
    // activity holds its own `ordinal + 1`. See [`activity_sequence`].
    let mut payloads: Vec<(usize, &str, Value)> = Vec::new();
    if let Some(message) = message {
        let turn = turn_reference(conversation, message);
        if let Some(bubble) = message_bubble(message, &turn, &at, origins) {
            payloads.push((
                0,
                MESSAGE_SENT,
                message_sent_payload(&conversation.task_id, &bubble),
            ));
        }
    }
    for activity in activities {
        let (turn, activity_at) = activity_anchor(conversation, window, activity);
        let (kind, payload) = activity_appended_payload(
            &conversation.task_id,
            activity_row(activity, &turn, &activity_at, window),
        );
        let slot = usize::try_from(activity_slot(activity)).unwrap_or(usize::MAX);
        payloads.push((slot.saturating_add(1), kind, payload));
    }

    // By slot, because a reader delivers an item only when its sequence is past
    // the last it delivered: a derived row takes a slot above the blocks, so it
    // is emitted between two of them and would cost the reader the one after it.
    payloads.sort_by_key(|(slot, _, _)| *slot);

    // `filter_map`, not `map_while`: the items are no longer numbered in list
    // order, so one past the ceiling says nothing about the next.
    payloads
        .into_iter()
        .filter_map(|(index, kind, payload)| {
            let sequence = change.sequence(index)?;
            Some(event(conversation, sequence, &at, kind, payload))
        })
        .collect()
}

/// The one work-log row a change names by activity id, as its event.
///
/// Numbered `ordinal + 1`, matching [`activity_sequence`], so a client sees the
/// same sequence whether the row arrived in a snapshot or here.
fn activity_change_events(
    conversation: &Conversation,
    change: &ConversationChange,
    window: &Transcript,
    activity_id: &str,
) -> Vec<Value> {
    let Some(activity) = window
        .activities
        .iter()
        .find(|activity| activity.activity_id == activity_id)
    else {
        return Vec::new();
    };
    let (turn, at) = activity_anchor(conversation, window, activity);
    let (kind, payload) = activity_appended_payload(
        &conversation.task_id,
        activity_row(activity, &turn, &at, window),
    );
    let slot = usize::try_from(activity_slot(activity)).unwrap_or(usize::MAX);
    change
        .sequence(slot.saturating_add(1))
        .map(|sequence| vec![event(conversation, sequence, &at, kind, payload)])
        .unwrap_or_default()
}

/// The conversation's session as an event, or nothing when it has none.
///
/// A change naming no Message is a change to the Turn or Sandbox; the contract
/// carries both inside the session, and a conversation with no Sandbox row has
/// no session to send.
fn session_set(conversation: &Conversation, change: &ConversationChange) -> Option<Value> {
    let session = session(conversation);
    if session.is_null() {
        return None;
    }
    let at = instant(
        conversation
            .updated_at
            .unwrap_or_else(|| created_at(conversation)),
    );
    let sequence = change.sequence(0)?;
    Some(event(
        conversation,
        sequence,
        &at,
        "thread.session-set",
        json!({ "threadId": conversation.task_id, "session": session }),
    ))
}

/// The cut a rewind made, as the event that applies it to a client's copy.
///
/// `turnCount` is the highest `task_turns.turn_number` kept; the client drops
/// every checkpoint, message, activity and plan above it. Slot 0, since a
/// rewind names no row to share it with.
fn revert_event(
    conversation: &Conversation,
    change: &ConversationChange,
    turn_count: i32,
) -> Option<Value> {
    let at = instant(
        conversation
            .updated_at
            .unwrap_or_else(|| created_at(conversation)),
    );
    let sequence = change.sequence(0)?;
    Some(event(
        conversation,
        sequence,
        &at,
        "thread.reverted",
        json!({ "threadId": conversation.task_id, "turnCount": turn_count.max(0) }),
    ))
}

const MESSAGE_SENT: &str = "thread.message-sent";

/// One work-log row as the event carrying it. The activity is the contract's
/// own shape already, so this only names the conversation it belongs to.
fn activity_appended_payload(task_id: &str, activity: Value) -> (&'static str, Value) {
    (
        "thread.activity-appended",
        json!({ "threadId": task_id, "activity": activity }),
    )
}

/// A message bubble as the event carrying it.
///
/// The contract spells one message two ways — a snapshot's `messages` member
/// and this payload; rewriting rather than re-deriving keeps the two from
/// drifting.
pub(super) fn message_sent_payload(task_id: &str, bubble: &Value) -> Value {
    let mut payload = json!({
        "threadId": task_id,
        "messageId": bubble["id"],
        "role": bubble["role"],
        "text": bubble["text"],
        "turnId": bubble["turnId"],
        "streaming": bubble["streaming"],
        "createdAt": bubble["createdAt"],
        "updatedAt": bubble["updatedAt"]
    });
    // Copied the same way as the seven above, so the delta and the snapshot are
    // one shape: a bubble with no origin produces a payload with none.
    if let Some(origin) = bubble.get("origin") {
        if let Some(object) = payload.as_object_mut() {
            object.insert("origin".into(), origin.clone());
        }
    }
    payload
}

/// One event, in the envelope every event on this contract carries.
///
/// `eventId` is `task_id:sequence`, so the same change re-read produces the
/// same id, and a client that sees one twice sees the same event.
pub(super) fn event(
    conversation: &Conversation,
    sequence: i64,
    occurred_at: &str,
    event_type: &str,
    payload: Value,
) -> Value {
    json!({
        "kind": "event",
        "event": {
            "sequence": sequence,
            "eventId": format!("{}:{sequence}", conversation.task_id),
            "aggregateKind": "thread",
            "aggregateId": conversation.task_id,
            "occurredAt": occurred_at,
            "commandId": Value::Null,
            "causationEventId": Value::Null,
            "correlationId": Value::Null,
            "metadata": json!({}),
            "type": event_type,
            "payload": payload
        }
    })
}
