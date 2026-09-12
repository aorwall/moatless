//! One Moatless scalar as the scalar the contract asks for.

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};

use crate::dao::Conversation;

pub(super) fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Both columns default to the insert time, so a missing value means a
/// hand-written row rather than an unknown instant.
pub(super) fn timestamp(value: Option<DateTime<Utc>>) -> String {
    instant(value.unwrap_or_else(Utc::now))
}

/// The one timestamp format on this wire.
pub(super) fn instant(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A nullable instant, for the contract's several `NullOr(IsoDateTime)` fields.
pub(super) fn nullable_instant(value: Option<DateTime<Utc>>) -> Value {
    value.map_or(Value::Null, |value| json!(instant(value)))
}

/// When the conversation was created.
///
/// `tasks` records this twice: as text the caller supplied, and as a database
/// timestamp added later. The typed column wins; the text is the fallback for
/// rows written before it existed.
pub(super) fn created_at(conversation: &Conversation) -> DateTime<Utc> {
    conversation
        .created_at_db
        .or_else(|| {
            DateTime::parse_from_rfc3339(&conversation.created_at)
                .ok()
                .map(|value| value.with_timezone(&Utc))
        })
        .unwrap_or_else(Utc::now)
}
