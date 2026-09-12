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
use crate::dao::{Conversation, Transcript};
use moatless_core::change_feed::conversation::ConversationChange;
use moatless_types::activity::Activity;

/// Everything one recorded change tells a client that already holds this
/// conversation, as stream items, in the order the client applies them.
///
/// The contract numbers events, so each item's sequence comes from the change's
/// position and its own index within the change — see
/// [`ConversationChange::sequence`]. That is also the ceiling on how much one
/// change may say: past [`EVENTS_PER_CHANGE`] the next position begins, and
/// this stops rather than colliding with it.
///
/// `window` is the changed Message and the short run before it
/// (`ConversationDao::changed_messages`); a change naming a Message the window
/// does not hold — deleted, or never rendered — says nothing, which is a
/// complete answer rather than a failure.
///
/// A change to one Message emits its bubble (`thread.message-sent`) and its
/// work-log rows (`thread.activity-appended`), both upserts keyed on their own
/// ids. A change naming a uuid with no Message row — a turn that only called
/// tools — emits the activities alone. A change naming an activity rather than a
/// Message emits that row alone and no `session_set`.
///
/// A rewind names neither, because the rows it is about are already deleted; it
/// carries [`ConversationChange::reverted_to_turn`] instead and emits
/// `thread.reverted`, which is the one event here that tells a client to drop
/// what it holds rather than to take something new.
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
/// A Sandbox lifecycle change and a context-window reading happen on no Message,
/// so the change names the activity itself and this is its whole projection: no
/// bubble, and no `session_set` — nothing about the conversation's own session
/// moved. The row is numbered `ordinal + 1` for the reason a Message's
/// activities are, so a client sees the same sequence whether the row arrived in
/// a snapshot (via [`activity_sequence`]) or here. A change naming a row the
/// window does not hold says nothing, which is a complete answer.
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
/// A change that names no Message is a change to the conversation itself — its
/// Turn or its Sandbox — and the contract carries both inside the session: the
/// client derives `latestTurn` from the session it is sent. A conversation with
/// no Sandbox row has no session at all, and the contract's payload has no
/// empty value for one, so there is nothing to say until it does.
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
/// `turnCount` is the highest `task_turns.turn_number` the conversation kept, and
/// the client drops every checkpoint above it along with that turn's messages,
/// activities and plans — the same number a [`checkpoints`] entry reports, so the
/// comparison it makes is against values this projection also produced.
///
/// Slot 0, like [`session_set`]: a rewind names no row, so nothing else shares
/// its position.
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
/// The contract spells one message two ways — a member of a snapshot's
/// `messages`, and this payload — with the same values under different keys.
/// Rewriting rather than re-deriving keeps the two from drifting: whatever a
/// snapshot would have shown is what an event says.
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
/// Five of the envelope's fields are null or empty and stay that way. Moatless
/// has no command bus: nothing here was caused by a numbered command, so
/// `commandId` and `correlationId` have nothing to name, and no event is
/// derived from another so `causationEventId` names nothing either. `metadata`
/// is provider bookkeeping this projection does not read.
///
/// `eventId` is the conversation and the sequence, because that pair is what
/// identifies an event here: the same change re-read produces the same ids, and
/// a client that sees one twice sees the same event rather than a new one.
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
