//! The transcript: one window of messages and activities as the client renders//! it.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::origin::OriginContext;
use super::origin::message_origin;
use super::scalar::created_at;
use super::scalar::instant;
use super::turn::turn_id;
use super::turn::turn_reference;
use crate::dao::Conversation;
use crate::transcript::{Transcript, TranscriptMessage};
use moatless_types::activity::Activity;
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
    // A Sandbox wake opens its Turn with no Message row, so the newest Turn
    // already started also counts; otherwise a question asked after a `moat
    // cmd` wake lands in the Turn before, which `request_status` reads as
    // expired.
    let started_by_then = window
        .turn_outcomes
        .iter()
        .map(|turn| (turn.turn_number, turn.started_at))
        .chain(
            conversation
                .turn_number
                .map(|n| (n, conversation.turn_started_at)),
        )
        .filter(|(_, started)| started.is_some_and(|at| at <= activity.created_at))
        .map(|(number, _)| number)
        .max();
    let number = match preceding.and_then(|message| message.turn_number) {
        Some(number) => Some(started_by_then.map_or(number, |started| number.max(started))),
        None => conversation.turn_number,
    };
    let turn = number.map_or(Value::Null, |number| {
        Value::String(turn_id(&conversation.task_id, number))
    });
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
