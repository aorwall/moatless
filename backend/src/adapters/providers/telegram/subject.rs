//! Canonical Telegram subject coordinates.
//!
//! Every Telegram destination is the same three facts — a chat, a message, and
//! an optional forum topic — read out of a JSON bag. The trait still hands that
//! bag over as a `&serde_json::Value` (spec Open question 3); this module is
//! where it stops being one.

use crate::adapters::error::{AdapterError, SharedAdapterError};
use crate::adapters::{AdapterDestinationBuilder, BindingDestination};

use super::adapter::TelegramAdapter;

/// The subject kind every Telegram binding is stored under.
pub(super) const SUBJECT_KIND: &str = "telegram_thread";

/// Where a Telegram message goes, read once out of the bag that carries it.
// reason: these are Telegram's own field names, and the bag is keyed by them —
// renaming the fields would hide the one-to-one mapping this type exists for.
#[allow(clippy::struct_field_names)]
pub(super) struct TelegramDestination {
    chat_id: String,
    message_id: Option<i64>,
    forum_topic_id: Option<i64>,
}

impl TelegramDestination {
    /// The bag [`super::webhook::normalize`] builds, whose `chat_id` the
    /// tracked-thread lookup path omits — `source_id` is the chat there.
    fn from_thread_properties(value: &serde_json::Value, source_id: &str) -> Self {
        Self {
            chat_id: value
                .get("chat_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(source_id)
                .to_string(),
            message_id: value.get("message_id").and_then(serde_json::Value::as_i64),
            forum_topic_id: value
                .get("forum_topic_id")
                .and_then(serde_json::Value::as_i64),
        }
    }

    /// A stored binding's `destination_config`, for an outbound path that has
    /// nowhere to send without a chat id.
    pub(super) fn from_destination_config(value: &serde_json::Value) -> Result<Self, AdapterError> {
        Self::parse(value).ok_or_else(|| {
            SharedAdapterError::BadRequest(
                "Telegram binding destination_config has no `chat_id` field".into(),
            )
            .into()
        })
    }

    /// The legacy thread-based reply path, which treats an absent chat id as an
    /// internal inconsistency rather than a bad request. The two error classes
    /// are load-bearing — one is a `400`, the other a `500` — so they stay
    /// distinct.
    pub(super) fn from_thread_context(value: &serde_json::Value) -> Result<Self, AdapterError> {
        Self::parse(value).ok_or_else(|| {
            SharedAdapterError::Internal("No chat_id in thread properties".into()).into()
        })
    }

    fn parse(value: &serde_json::Value) -> Option<Self> {
        let chat_id = value
            .get("chat_id")
            .and_then(serde_json::Value::as_str)?
            .to_string();
        Some(Self {
            chat_id,
            message_id: value.get("message_id").and_then(serde_json::Value::as_i64),
            forum_topic_id: value
                .get("forum_topic_id")
                .and_then(serde_json::Value::as_i64),
        })
    }

    pub(super) fn chat_id(&self) -> &str {
        &self.chat_id
    }

    pub(super) const fn message_id(&self) -> Option<i64> {
        self.message_id
    }

    pub(super) const fn forum_topic_id(&self) -> Option<i64> {
        self.forum_topic_id
    }

    /// A forum topic is its own subject; everything else is keyed by the thread
    /// the caller resolved.
    fn subject_key(&self, external_thread_id: &str) -> String {
        let chat_id = &self.chat_id;
        match self.forum_topic_id {
            Some(topic_id) => format!("{chat_id}:topic:{topic_id}"),
            None => format!("{chat_id}:thread:{external_thread_id}"),
        }
    }

    fn destination_config(&self) -> serde_json::Value {
        serde_json::json!({
            "chat_id": self.chat_id,
            "message_id": self.message_id,
            "forum_topic_id": self.forum_topic_id,
        })
    }
}

impl AdapterDestinationBuilder for TelegramAdapter {
    fn binding_destination_from_existing_thread(
        &self,
        source_id: &str,
        external_thread_id: &str,
        thread_properties: &serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, AdapterError> {
        let destination = TelegramDestination::from_thread_properties(thread_properties, source_id);
        let mut destination_config = destination.destination_config();
        if let Some(filter_user_id) = filter_user_id {
            destination_config["filter_user_id"] = serde_json::json!(filter_user_id);
        }

        Ok(BindingDestination::new(
            SUBJECT_KIND,
            destination.subject_key(external_thread_id),
            destination_config,
        ))
    }
}

/// The chat id in a `chat:<id>` direct destination.
pub(super) fn parse_direct_destination(destination: &str) -> Result<&str, AdapterError> {
    let raw = destination.trim();
    let chat_id = raw.strip_prefix("chat:").ok_or_else(|| {
        SharedAdapterError::BadRequest("Telegram destination must be `chat:<id>`".into())
    })?;
    let chat_id = chat_id.trim();
    if chat_id.is_empty() {
        return Err(SharedAdapterError::BadRequest(
            "Telegram destination must include a chat id".into(),
        )
        .into());
    }
    Ok(chat_id)
}

/// The `tg:m:<id>` message a reply or reaction targets.
pub(super) fn parse_message_target(
    target_external_id: &str,
    what: &str,
) -> Result<i64, AdapterError> {
    super::external_id::message_id_from(target_external_id).ok_or_else(|| {
        SharedAdapterError::BadRequest(format!(
            "Telegram {what} target `{target_external_id}` is not a tg:m:<id> id"
        ))
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_chat_id_survives_the_trip_through_a_string() {
        let chat_id: i64 = -1_001_234_567_890;
        assert_eq!(chat_id.to_string(), "-1001234567890");
    }

    #[test]
    fn binding_destination_from_existing_thread_uses_topic_subject_when_present() {
        let provider = TelegramAdapter::new("fake_token");
        let destination = provider
            .binding_destination_from_existing_thread(
                "-100123",
                "77",
                &serde_json::json!({
                    "chat_id": "-100123",
                    "message_id": 77,
                    "forum_topic_id": 45,
                }),
                Some("user-1"),
            )
            .unwrap();

        assert_eq!(destination.subject_kind, "telegram_thread");
        assert_eq!(destination.subject_key, "-100123:topic:45");
        assert_eq!(destination.destination_config["chat_id"], "-100123");
        assert_eq!(destination.destination_config["message_id"], 77);
        assert_eq!(destination.destination_config["forum_topic_id"], 45);
        assert_eq!(destination.destination_config["filter_user_id"], "user-1");
    }

    #[test]
    fn a_direct_destination_must_name_a_chat() {
        assert_eq!(parse_direct_destination(" chat: 999 ").unwrap(), "999");

        for (destination, fragment) in [("999", "chat:<id>"), ("chat:", "include a chat id")] {
            let error = parse_direct_destination(destination)
                .expect_err("a destination without a chat id is refused");
            assert!(error.to_string().contains(fragment), "{error}");
        }
    }

    #[test]
    fn a_message_target_must_be_a_telegram_message_id() {
        assert_eq!(parse_message_target("tg:m:42", "reaction").unwrap(), 42);

        let error = parse_message_target("42", "reply").expect_err("a bare id is refused");
        assert!(
            error.to_string().contains("Telegram reply target `42`"),
            "{error}"
        );
    }

    /// The two absent-chat errors are different classes on purpose: an outbound
    /// binding with no chat is a bad request, a thread context with none is a
    /// backend inconsistency.
    #[test]
    fn an_absent_chat_id_is_a_bad_request_outbound_and_internal_on_the_thread_path() {
        let empty = serde_json::json!({});

        assert!(matches!(
            TelegramDestination::from_destination_config(&empty),
            Err(AdapterError::Shared(SharedAdapterError::BadRequest(_)))
        ));
        assert!(matches!(
            TelegramDestination::from_thread_context(&empty),
            Err(AdapterError::Shared(SharedAdapterError::Internal(_)))
        ));
    }

    #[test]
    fn a_thread_properties_bag_without_a_chat_falls_back_to_the_source() {
        let destination =
            TelegramDestination::from_thread_properties(&serde_json::json!({}), "-100123");

        assert_eq!(destination.chat_id(), "-100123");
        assert_eq!(destination.subject_key("77"), "-100123:thread:77");
    }
}
