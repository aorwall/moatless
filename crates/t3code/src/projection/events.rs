//! The change events a subscription sends between snapshots.

use serde_json::{Value, json};

use super::detail::checkpoints;
use super::detail::is_plan_call;
use super::detail::proposed_plan;
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
use crate::transcript::Transcript;
use moatless_core::change_feed::conversation::ConversationChange;
use moatless_types::activity::{Activity, ActivityKind};

/// Turns one recorded change into stream items, in the order a client applies
/// them.
///
/// Sequences are capped at [`EVENTS_PER_CHANGE`] per change, a Message outside
/// `window` says nothing, and a plan also emits its card ([`plan_cards`]).
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
    for activity in &activities {
        let (turn, activity_at) = activity_anchor(conversation, window, activity);
        let (kind, payload) = activity_appended_payload(
            &conversation.task_id,
            activity_row(activity, &turn, &activity_at, window),
        );
        let slot = usize::try_from(activity_slot(activity)).unwrap_or(usize::MAX);
        payloads.push((slot.saturating_add(1), kind, payload));
    }
    for (ordinal, call) in plan_cards(&activities, window) {
        if let Some(plan) = proposed_plan(conversation, window, call) {
            let slot = usize::try_from(plan_card_slot(ordinal)).unwrap_or(usize::MAX);
            payloads.push((
                slot.saturating_add(1),
                PROPOSED_PLAN_UPSERTED,
                proposed_plan_payload(&conversation.task_id, plan),
            ));
        }
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
        .map_or_default(|sequence| vec![event(conversation, sequence, &at, kind, payload)])
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

/// Each of `settled`'s turns' checkpoints, with its turn number, sent live so
/// the rewind control reaches those turns without a reload.
///
/// Slots 1 onward: the client must settle `latestTurn` off slot 0's
/// `thread.session-set` first.
pub(crate) fn turn_settled_events(
    conversation: &Conversation,
    change: &ConversationChange,
    settled: &Transcript,
) -> Vec<(i32, Value)> {
    checkpoints(conversation, settled)
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut payload)| {
            let turn = i32::try_from(payload["checkpointTurnCount"].as_i64()?).ok()?;
            let at = payload["completedAt"].as_str()?.to_owned();
            payload
                .as_object_mut()?
                .insert("threadId".into(), json!(conversation.task_id));
            let sequence = change.sequence(index + 1)?;
            let event = event(
                conversation,
                sequence,
                &at,
                "thread.turn-diff-completed",
                payload,
            );
            Some((turn, event))
        })
        .collect()
}

/// Whether `change` moved the conversation's own session rather than naming a
/// row, which is the change a turn settling records.
pub(crate) fn is_session_change(change: &ConversationChange) -> bool {
    change.message_uuid.is_none()
        && change.activity_id.is_none()
        && change.reverted_to_turn.is_none()
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

/// The plan calls this change carries a card for, each with the ordinal of the
/// row that called for one.
///
/// The call itself when the agent proposes, the `UserInputResolved` row when an
/// answer lands — that row never shares a change with the call it settles.
fn plan_cards<'a>(activities: &[&'a Activity], window: &'a Transcript) -> Vec<(i32, &'a Activity)> {
    let mut cards = Vec::new();
    for activity in activities {
        if is_plan_call(activity) {
            cards.push((activity.ordinal, *activity));
            continue;
        }
        if activity.kind != ActivityKind::UserInputResolved {
            continue;
        }
        let request_id = activity.payload.get("requestId").and_then(Value::as_str);
        let call = request_id.and_then(|request_id| {
            window
                .activities
                .iter()
                .find(|call| is_plan_call(call) && call.tool_use_id.as_deref() == Some(request_id))
        });
        if let Some(call) = call {
            cards.push((activity.ordinal, call));
        }
    }
    cards
}

/// The slot a plan card owns, from the ordinal of the row in this change it came
/// from, so two cards cannot land on one slot.
///
/// Above the band [`activity_slot`] gives the answer panel, since a call, its
/// panel and its card are three events off one row.
fn plan_card_slot(ordinal: i32) -> i64 {
    PLAN_CARD_SLOT_BASE + i64::from(ordinal)
}

/// Where a plan card's slots begin, and the ceiling on the band below it.
const PLAN_CARD_SLOT_BASE: i64 = 768;

const MESSAGE_SENT: &str = "thread.message-sent";
const PROPOSED_PLAN_UPSERTED: &str = "thread.proposed-plan-upserted";

/// One proposed plan as the event carrying it. The plan is the contract's own
/// shape already, so this only names the conversation it belongs to.
fn proposed_plan_payload(task_id: &str, plan: Value) -> Value {
    json!({ "threadId": task_id, "proposedPlan": plan })
}

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
