//! The transcript: one window of messages and activities as the client renders it.

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
use crate::dao::{Conversation, Transcript, TranscriptMessage};
use moatless_core::change_feed::conversation::EVENTS_PER_CHANGE;
use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityView;
use moatless_types::message::MessageType;

const TOOL_RESPONSE_USER_TYPE: &str = "tool_response";

/// What `ChatImageAttachment` and `ChatFileAttachment` accept, field for field
/// (`contracts/src/orchestration.ts:186-205`). An attachment is projected as
/// whichever member it fits, and one that fits neither is left out rather than
/// sent for the client to refuse — see [`message_attachments`].
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
/// T3 splits a conversation the way a reader does and Moatless does not: what
/// somebody *said* is a message, and what the agent *did* on the way to saying
/// it is an activity the client renders as a collapsible work log. One Moatless
/// Message can therefore produce a message and several activities, or only
/// activities — an assistant turn that did nothing but call tools has no text
/// and contributes no bubble.
pub(super) struct RenderedTranscript {
    pub(super) messages: Vec<Value>,
    pub(super) activities: Vec<Value>,
}

/// Split a Task's stored rows into the messages and activities T3 renders.
///
/// Two walks rather than one, because the two collections no longer nest: an
/// activity belongs to a Message by provenance, not by containment, and a turn
/// that only called tools has activities and no Message at all. Ordering is
/// carried explicitly on each side — Message order for the bubbles, the
/// conversation's own `position` for the work log.
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
/// The meter shows one number, and the client reads it off the newest reading it
/// holds — every earlier one in the same turn is a row nothing renders. A
/// harness reporting per model request can produce several a minute, and this
/// projection re-sends its whole log every time the conversation moves, so the
/// superseded ones are dropped on the way out rather than stored differently.
///
/// Per turn rather than per conversation, because a Turn is the unit a person
/// scrolls back to: the reading a finished turn ended on stays legible beside it.
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
/// rather than what it holds, which is why this is a match on the type rather
/// than a filter on the text.
pub(super) fn message_bubble(
    message: &TranscriptMessage,
    turn: &Value,
    at: &str,
    origins: &OriginContext<'_>,
) -> Option<Value> {
    let text = message.message_text.as_deref().unwrap_or_default();
    match message.message_type {
        // A leftover carrier row for tool results. Nothing writes these any
        // more, and what they carried is on the activity the call wrote.
        MessageType::System => None,
        MessageType::User => {
            // A `user` row of this kind is the person answering a tool the agent
            // used to ask them something — the answer already reaches the client
            // as that call's outcome, and `message/transform.rs` makes the same
            // exclusion for the same reason.
            if message.user_type == TOOL_RESPONSE_USER_TYPE {
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
        // A run's own closing line, from before those stopped being written as
        // Messages. Neither the person nor the agent said it, and `system` is
        // the contract's role for exactly that.
        MessageType::Result | MessageType::Error => (!text.is_empty())
            .then(|| message_row(message, "system", text.to_string(), turn, at, None)),
    }
}

/// The uploads on a Message, as the contract's `ChatAttachment` union.
///
/// Which member an upload becomes is decided by which one it *fits*, not by
/// what its mime type looks like. A `ChatImageAttachment` is one of the four
/// types the image member names — gif, jpeg, png, webp — at up to 10 MiB, and
/// may be zero bytes; a `ChatFileAttachment` takes any type at one byte to
/// 50 MiB (`contracts/src/orchestration.ts:186-205`). So an upload the image
/// member cannot hold, because it is an SVG or because it is 20 MiB, is a file
/// rather than nothing: the mint accepts both under `type: "file"`, and the
/// file member is exactly the shape the contract has for them.
///
/// An upload past *both* members is left out rather than sent, because the
/// client decodes the *whole* thread snapshot against that schema: a single
/// field that fails its check costs the reader the conversation, not the
/// attachment. Leaving it out costs them one row in one bubble. Reading the
/// member off the mime type alone got this wrong in both directions at once —
/// dropping a large image the agent had already read, and sending an image
/// member for a type the client has no shape for.
///
/// The agent is unaffected either way — it reads the file off disk from the
/// path the prompt names, which is decided by the upload and not by this.
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
            // The member is whichever one the attachment fits, not the one its
            // mime type suggests, and fitting the image member takes more than
            // an `image/` prefix: the contract names four types, so an SVG or
            // an AVIF is a file however image-like it reads. The file member
            // carries any type up to 50 MiB, which is what both of those cases
            // land in — a 20 MiB PNG sent as a file, and a 1 KiB SVG.
            //
            // Getting this wrong in either direction costs the whole
            // conversation rather than the attachment: too strict and an upload
            // the mint accepted goes missing after the agent has read it, too
            // loose and the client is sent an image member it has no shape for.
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
/// migrated id from reaching a client that would refuse the snapshot holding it.
fn is_attachment_id(id: &str) -> bool {
    id.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Cut `text` to at most `max` bytes without splitting a character.
///
/// Bytes rather than characters, and deliberately: the contract's limit counts
/// UTF-16 units, which no multi-byte character can exceed the byte count of, so
/// a byte cut is never the looser reading of the same rule.
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
/// An activity carries no turn of its own — a turn is a property of the
/// conversation, and the row records what happened, not when the bookkeeping
/// around it changed. So the Turn is borrowed, in three steps:
///
/// 1. From the Message the activity was written with, when that Message exists.
/// 2. From the newest Message before it, when it does not — a turn that only
///    called tools wrote no Message, and its calls belong with the turn already
///    under way.
/// 3. From the conversation's newest Turn, when the window holds no Message at
///    all. This is the live path: a change is read moments after it was
///    recorded, so the newest Turn is the one it happened in.
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
    let preceding = window.messages.iter().rev().find(|message| {
        message
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
        // `task_messages.uuid` is unique across the deployment and is the
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
/// The stored payload owns the title, status and complete call data. The
/// canonical activity view projects that into this client's compact shape. The
/// preview is stored too, in its own column, cut once when the result arrived
/// rather than on every send. Full `input`, `result` and `typedResult` stay in
/// storage for the Moatless frontend, which asks for one activity at a time;
/// this contract re-sends its whole snapshot whenever the conversation moves.
///
/// `sequence` is the number this row's own event carries — see
/// [`activity_sequence`]. An activity delivered on its own, long after the
/// snapshot it has to sort into, carries the number it was written with.
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
/// Stamped here rather than in the payload projection because the numbers are
/// not on the row: they are summed across Messages the transcript excludes, and
/// reach this point on the transcript that read them. Keyed by the `taskId` the
/// payload projection already stamped, so a row that names no subagent — every
/// row but one kind — is left alone.
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
/// The row a panel is derived from is written beside the call rather than being
/// it, so no result ever lands on it and its own status never moves. Left at
/// `requested` it asks forever: the client keeps the panel open until it sees a
/// row naming the same request resolved, and an answer to a settled call is
/// refused, so the question cannot be got rid of from there at all.
///
/// The call's status is what settles it, however it was settled — answered here,
/// answered on another surface, or dropped by a resuming session. This reads it
/// the way a tool row already reads its own: open, it is a request; come back, it
/// is a resolution.
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
/// `position * EVENTS_PER_CHANGE + ordinal + 1` — the number
/// [`ConversationChange::sequence`] computes for the same row, so a client sees
/// the same value whether it arrived in a snapshot or as an event. That is what
/// the invariant at `task_activities::models` asks for: "a snapshot's sequence
/// and an event's sequence have to be the same kind of number for that to mean
/// anything".
///
/// **Slot 0 is the Message's bubble, whether or not one is emitted.** A change
/// enumerates a list that begins with the bubble only when the Message produced
/// one, so an index counted while walking would shift by one depending on
/// whether the caller's slice held that Message at all. Reserving the slot
/// removes the dependency; a change with nothing to say leaves a gap, and a
/// sequence is a cursor rather than a count.
///
/// The `+1` lives here rather than in the stored column: folding it in would
/// force either a special case for the rows that name no Message or the rule
/// written twice.
pub(super) fn activity_sequence(activity: &Activity) -> i64 {
    activity.position * EVENTS_PER_CHANGE + activity_slot(activity) + 1
}

/// The slot a row owns inside its change, above the bubble's slot 0.
///
/// A row a panel is derived from is written beside the call at the same Message
/// and the same block, so the block alone numbers the two identically — and a
/// reader that delivers each sequence once keeps whichever arrived first, always
/// the call, because it sorts ahead of the row derived from it. The panel row is
/// then dropped on the way out, which is every appearance and every closure of
/// that panel.
///
/// So derived rows take a band above every block a Message can hold. Read off
/// the row rather than stored on it, so rows already written are numbered by the
/// same rule, and a snapshot and a live change go on agreeing.
pub(super) fn activity_slot(activity: &Activity) -> i64 {
    let ordinal = i64::from(activity.ordinal);
    match activity.kind {
        ActivityKind::UserInputRequested | ActivityKind::PlanUpdated => DERIVED_SLOT_BASE + ordinal,
        _ => ordinal,
    }
}

/// Where the derived rows' slots begin. Above any block count a Message reaches,
/// and low enough that the band itself stays inside [`EVENTS_PER_CHANGE`].
const DERIVED_SLOT_BASE: i64 = 512;

/// When a Message happened.
///
/// `timestamp` is what the writer said, and is what the transcript is ordered
/// by, so it is also what the transcript reports — normalized, because the
/// client compares these as strings and a row in another format would sort
/// away from its neighbours. A row whose `timestamp` will not parse falls back
/// to when the database recorded it, and then to the conversation's own start.
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
