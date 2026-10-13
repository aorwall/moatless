//! The transcript: one window of messages and activities as the client renders
//! it.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use super::detail::elision_notice;
use super::origin::OriginContext;
use super::origin::message_origin;
use super::scalar::created_at;
use super::scalar::instant;
use super::turn::turn_id;
use super::turn::turn_reference;
use crate::activity::T3ActivityViewExt;
use crate::dao::Conversation;
use crate::transcript::{Transcript, TranscriptMessage};
use moatless_core::change_feed::conversation::EVENTS_PER_CHANGE;
use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityView;
use moatless_types::message::MessageType;
use moatless_types::message::TOOL_RESPONSE_USER_TYPE;

/// `ChatAttachmentId`'s max length (`contracts/src/orchestration.ts:186-205`).
///
/// An attachment past this, or matching neither member's shape, is left out
/// rather than sent for the client to refuse — see [`message_attachments`].
const ATTACHMENT_ID_MAX_CHARS: usize = 128;

pub(super) const ATTACHMENT_NAME_MAX_CHARS: usize = 255;

const ATTACHMENT_MIME_MAX_CHARS: usize = 100;

/// The same ceilings the mint signs for and the resolver measures a referenced
/// upload against, so what this surface accepts and what it projects back
/// cannot drift apart.
pub(super) use crate::attachments::MAX_FILE_BYTES as ATTACHMENT_MAX_FILE_BYTES;
pub(super) use crate::attachments::MAX_IMAGE_BYTES as ATTACHMENT_MAX_IMAGE_BYTES;
use crate::attachments::SUPPORTED_IMAGE_MIME_TYPES;

/// The two collections a conversation's transcript becomes.
///
/// T3 splits what somebody *said* (a message) from what the agent *did* to say
/// it (activities); one Moatless Message can therefore produce both, or only
/// activities.
pub(super) struct RenderedTranscript {
    pub(super) messages: Vec<Value>,
    pub(super) activities: Vec<Value>,
}

/// Split a Task's stored rows into the messages and activities T3 renders.
///
/// Two walks, not one: an activity belongs to a Message by provenance, not
/// containment, and a turn that only called tools has activities and no Message
/// at all.
pub(super) fn rendered_transcript(
    conversation: &Conversation,
    transcript: &Transcript,
    origins: &OriginContext<'_>,
) -> RenderedTranscript {
    let mut rendered = RenderedTranscript {
        messages: Vec::new(),
        activities: Vec::new(),
    };
    if transcript.omitted > 0 {
        rendered
            .messages
            .push(elision_notice(conversation, transcript));
    }

    for message in &transcript.messages {
        let at = instant(message_instant(conversation, message));
        let turn = turn_reference(conversation, message);
        rendered
            .messages
            .extend(message_bubble(message, &turn, &at, origins));
    }
    let surviving = surviving_context_windows(transcript);
    for activity in &transcript.activities {
        if activity.kind == ActivityKind::ContextWindowUpdated
            && !surviving.contains(activity.activity_id.as_str())
        {
            continue;
        }
        let (turn, at) = activity_anchor(conversation, transcript, activity);
        rendered
            .activities
            .push(activity_row(activity, &turn, &at, transcript));
    }

    rendered
}

/// Which context-window readings a snapshot carries: the newest per turn.
///
/// The client reads the meter off the newest reading it holds, so earlier ones
/// in the same turn render nothing; per turn, not per conversation, so a
/// finished turn's ending reading stays legible beside it.
fn surviving_context_windows(transcript: &Transcript) -> HashSet<&str> {
    let mut newest: std::collections::HashMap<Value, &Activity> = std::collections::HashMap::new();
    for activity in &transcript.activities {
        if activity.kind != ActivityKind::ContextWindowUpdated {
            continue;
        }
        let turn = transcript
            .messages
            .iter()
            .rev()
            .find(|message| {
                message
                    .created_at
                    .is_some_and(|created| created <= activity.created_at)
            })
            .and_then(|message| message.turn_number)
            .map_or(Value::Null, |number| json!(number));
        newest
            .entry(turn)
            .and_modify(|held| {
                if (held.position, held.ordinal) <= (activity.position, activity.ordinal) {
                    *held = activity;
                }
            })
            .or_insert(activity);
    }
    newest
        .into_values()
        .map(|activity| activity.activity_id.as_str())
        .collect()
}

/// The bubble one Message contributes to the conversation, or `None` when it
/// contributes none.
///
/// Three of the four ways to contribute nothing are about *which* Message it is
/// rather than what it holds, so this matches on the type.
pub(super) fn message_bubble(
    message: &TranscriptMessage,
    turn: &Value,
    at: &str,
    origins: &OriginContext<'_>,
) -> Option<Value> {
    let text = message.message_text.as_deref().unwrap_or_default();
    match message.message_type {
        // A legacy carrier row for tool results: what it carried is on the
        // activity the call wrote.
        MessageType::System => None,
        MessageType::User => {
            // A `user` row of this kind is the person answering a tool the
            // agent asked them with, and the answer already reaches the client
            // as that call's outcome. `message/transform.rs` excludes it too.
            if message.user_type == TOOL_RESPONSE_USER_TYPE {
                return None;
            }
            // The sandbox waking its own agent: the agent's reply shows, the
            // wake itself is harness plumbing nobody said.
            if message.sandbox_wake {
                return None;
            }
            // Only a `user` bubble can carry an origin: an external message is
            // someone else speaking, and the agent's own replies are not.
            (!text.is_empty()).then(|| {
                let mut row = message_row(
                    message,
                    "user",
                    text.to_string(),
                    turn,
                    at,
                    message_origin(message, origins),
                );
                // Only a `user` bubble can carry attachments: the uploads on a
                // Message are what the person attached to it.
                let attachments = message_attachments(message);
                if !attachments.is_empty() {
                    if let Some(object) = row.as_object_mut() {
                        object.insert("attachments".into(), Value::Array(attachments));
                    }
                }
                row
            })
        }
        MessageType::Assistant => (!text.is_empty())
            .then(|| message_row(message, "assistant", text.to_string(), turn, at, None)),
        // A run's own closing line: neither the person nor the agent said it,
        // and `system` is the contract's role for exactly that.
        MessageType::Result | MessageType::Error => (!text.is_empty())
            .then(|| message_row(message, "system", text.to_string(), turn, at, None)),
    }
}

/// The uploads on a Message, as the contract's `ChatAttachment` union.
///
/// Decided by which member it *fits*, not by mime type name; an upload past
/// both is left out rather than sent, since a snapshot field that fails its
/// schema costs the reader the whole conversation.
pub(super) fn message_attachments(message: &TranscriptMessage) -> Vec<Value> {
    let Some(contexts) = message.file_contexts.as_ref().and_then(Value::as_array) else {
        return Vec::new();
    };

    contexts
        .iter()
        .filter_map(|context| {
            let id = context.get("referenceId").and_then(Value::as_str)?;
            if id.is_empty() || id.len() > ATTACHMENT_ID_MAX_CHARS || !is_attachment_id(id) {
                return None;
            }

            let data = context.get("data")?;
            let mime_type = data.get("mimeType").and_then(Value::as_str)?;
            if mime_type.is_empty() || mime_type.len() > ATTACHMENT_MIME_MAX_CHARS {
                return None;
            }
            let name = truncate_on_char_boundary(
                data.get("filename").and_then(Value::as_str)?.trim(),
                ATTACHMENT_NAME_MAX_CHARS,
            );
            if name.is_empty() {
                return None;
            }

            let size = data.get("size").and_then(Value::as_i64)?;
            let lowered = mime_type.to_ascii_lowercase();
            let kind = if SUPPORTED_IMAGE_MIME_TYPES.contains(&lowered.as_str())
                && (0..=ATTACHMENT_MAX_IMAGE_BYTES).contains(&size)
            {
                "image"
            } else if (1..=ATTACHMENT_MAX_FILE_BYTES).contains(&size) {
                "file"
            } else {
                return None;
            };

            Some(json!({
                "type": kind,
                "id": id,
                "name": name,
                "mimeType": mime_type,
                "sizeBytes": size
            }))
        })
        .collect()
}

/// `ChatAttachmentId`'s alphabet (`contracts/src/orchestration.ts:152-155`). An
/// upload id is a uuid and always passes; this is what keeps a hand-written or
/// migrated id from reaching a client that would refuse the snapshot holding
/// it.
fn is_attachment_id(id: &str) -> bool {
    id.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Cut `text` to at most `max` bytes without splitting a character.
///
/// Deliberately bytes, not characters: the contract's limit counts UTF-16
/// units, which no multi-byte character exceeds the byte count of, so a byte
/// cut is never looser than the rule.
fn truncate_on_char_boundary(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The Turn an activity is shown under, and when it happened.
///
/// An activity carries no turn of its own: borrowed from the Message it was
/// written with, else the newest Message before it that a Turn has taken,
/// else the conversation's newest Turn when there is none (the live path, read
/// moments after recording).
pub(super) fn activity_anchor(
    conversation: &Conversation,
    window: &Transcript,
    activity: &Activity,
) -> (Value, String) {
    if let Some(message) = activity
        .message_uuid
        .as_deref()
        .and_then(|uuid| window.messages.iter().find(|message| message.uuid == uuid))
    {
        return (
            turn_reference(conversation, message),
            instant(message_instant(conversation, message)),
        );
    }
    // A held Message, such as a child's notice, has its row from the moment it
    // is sent, so it is skipped until a Turn takes it and then while that Turn
    // started after the activity; otherwise a question lands in another Turn,
    // which `request_status` reads as expired. Live reads carry no
    // `turn_outcomes`, but the notice before a fresh activity is still held.
    let started_after = |number: i32| {
        window
            .turn_outcomes
            .iter()
            .find(|turn| turn.turn_number == number)
            .and_then(|turn| turn.started_at)
            .is_some_and(|started| started > activity.created_at)
    };
    let preceding = window.messages.iter().rev().find(|message| {
        message
            .turn_number
            .is_some_and(|number| !started_after(number))
            && message
                .created_at
                .is_some_and(|created| created <= activity.created_at)
    });
    let turn = match preceding {
        Some(message) => turn_reference(conversation, message),
        None => conversation.turn_number.map_or(Value::Null, |number| {
            Value::String(turn_id(&conversation.task_id, number))
        }),
    };
    (turn, instant(activity.created_at))
}

/// One message bubble.
fn message_row(
    message: &TranscriptMessage,
    role: &str,
    text: String,
    turn: &Value,
    at: &str,
    origin: Option<Value>,
) -> Value {
    let mut row = json!({
        // A Message's uuid is unique across the deployment and is the
        // identity every other Moatless surface already refers to a Message by.
        "id": message.uuid,
        "role": role,
        "text": text,
        "turnId": turn,
        // Moatless persists a Message once it is whole; there is no partial row
        // to report as still arriving.
        "streaming": false,
        "createdAt": at,
        // No column records a later edit, and none happens: the in-place
        // updates this table takes never touch content.
        "updatedAt": at
    });
    // Present only on a Message that came from outside T3. An ordinary bubble
    // carries no `origin` key at all, byte-identical to before this field.
    if let Some(origin) = origin {
        if let Some(object) = row.as_object_mut() {
            object.insert("origin".into(), origin);
        }
    }
    row
}

/// One stored activity as the work-log row the contract carries.
///
/// The preview is cut once, in its own column, when the result arrived, rather
/// than on every send; full `input`/`result`/`typedResult` stay in storage
/// since this contract re-sends its whole snapshot on every move. `sequence`
/// matches [`activity_sequence`].
pub(super) fn activity_row(
    activity: &Activity,
    turn: &Value,
    at: &str,
    window: &Transcript,
) -> Value {
    let view = ActivityView::new(activity);
    let mut payload = view.t3_payload();
    stamp_subagent_usage(&mut payload, window);

    json!({
        "id": view.activity_id(),
        "tone": view.t3_tone(),
        "kind": panel_kind(activity, &window.settled_requests).unwrap_or_else(|| view.t3_kind()),
        "summary": view.t3_summary(),
        "payload": Value::Object(payload),
        "turnId": turn,
        "sequence": activity_sequence(activity),
        "createdAt": at
    })
}

/// What the subagent this row spawned has spent, on the row that names it.
///
/// Stamped here, not in the payload projection, because the numbers are summed
/// across Messages the transcript excludes; keyed by the `taskId` already
/// stamped, so a row naming no subagent is left alone.
fn stamp_subagent_usage(payload: &mut Map<String, Value>, window: &Transcript) {
    let Some(usage) = payload
        .get("taskId")
        .and_then(Value::as_str)
        .and_then(|task_id| window.subagent_usage.get(task_id))
    else {
        return;
    };
    payload.insert(
        "typedUsage".into(),
        json!({
            "totalTokens": usage.input_tokens + usage.output_tokens,
            "inputTokens": usage.input_tokens,
            "outputTokens": usage.output_tokens
        }),
    );
}

/// The requests whose call has come back — [`Transcript::settled_requests`].
type SettledCalls = HashSet<String>;

/// What a question's panel row is, once its call has come back.
///
/// The panel row is written beside the call, not as it, so its own status never
/// moves; left at `requested` the client would keep it open forever, however
/// the call was actually settled.
fn panel_kind(activity: &Activity, settled: &SettledCalls) -> Option<&'static str> {
    if activity.kind != ActivityKind::UserInputRequested {
        return None;
    }
    let request_id = activity.payload.get("requestId").and_then(Value::as_str)?;
    settled
        .contains(request_id)
        .then_some(crate::activity::USER_INPUT_RESOLVED_KIND)
}

/// Where one activity sits in the conversation's event numbering.
///
/// Matches what [`ConversationChange::sequence`] computes for the same row, so
/// a client sees the same value whether it arrived in a snapshot or as an
/// event; slot 0 is always the Message's bubble, whether or not one is emitted.
pub(super) fn activity_sequence(activity: &Activity) -> i64 {
    activity.position * EVENTS_PER_CHANGE + activity_slot(activity) + 1
}

/// The slot a row owns inside its change, above the bubble's slot 0.
///
/// A panel row is written beside its call at the same block, so block-only
/// numbering would collide and drop it; derived rows take a band above every
/// block a Message can hold. Read off the row, not stored, so old and new rows
/// agree.
pub(super) fn activity_slot(activity: &Activity) -> i64 {
    let ordinal = i64::from(activity.ordinal);
    match activity.kind {
        ActivityKind::UserInputRequested | ActivityKind::PlanUpdated => DERIVED_SLOT_BASE + ordinal,
        _ => ordinal,
    }
}

/// Where the derived rows' slots begin. Above any block count a Message
/// reaches, and below `PLAN_CARD_SLOT_BASE`, which is where the band stops: a
/// Message holding 256 blocks or more puts a derived row on a plan card's slot.
const DERIVED_SLOT_BASE: i64 = 512;

/// When a Message happened.
///
/// `timestamp` is what the transcript is ordered by, normalized because the
/// client compares these as strings; a row whose `timestamp` will not parse
/// falls back to when the database recorded it, then to the conversation's
/// start.
pub(super) fn message_instant(
    conversation: &Conversation,
    message: &TranscriptMessage,
) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&message.timestamp)
        .map(|parsed| parsed.with_timezone(&Utc))
        .ok()
        .or(message.created_at)
        .unwrap_or_else(|| created_at(conversation))
}
