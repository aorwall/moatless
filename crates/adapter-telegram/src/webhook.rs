//! Raw Telegram update normalization.
//!
//! This module decides what one update means and returns it. Resolving the
//! sender to a system user is the only I/O it performs: there is no Bot API
//! call on any path and nothing is written, so every ignore reason below is
//! provable without a client and most of them without a database.
//!
//! What the update *causes* — the `/start` and `/help` answers, the pairing
//! request — comes back as [`Effect`] values for the caller to apply.

use tracing::{debug, info};

use moatless_loops::adapters::AdapterKind;
use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::models::AccessDecision;
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::LoopContext;
use moatless_backend_core::identity::ProviderIdentityLink;

use super::access::{self, DmDecision, DmSender};
use super::config::TelegramAdapterConfig;

/// What ingress already knows by the time it normalizes.
pub(super) struct NormalizeContext<'a> {
    pub(super) config: &'a LoopContext,
    pub(super) tg_config: &'a TelegramAdapterConfig,
    pub(super) users: &'a dyn ProviderIdentityLink,
    pub(super) is_private_chat: bool,
}

/// What one update normalized to.
pub(super) enum Normalization {
    Message {
        message: Box<InboundMessage>,
        effects: Vec<Effect>,
    },
    Ignored {
        /// Why, for the log. The webhook metric keeps its single
        /// `not_routable` reason (spec Decision 9).
        reason: &'static str,
        effects: Vec<Effect>,
    },
}

/// Something the update asks the Bot API or the database for, which
/// normalization decided on but did not do.
pub(super) enum Effect {
    /// `/start` and `/help`, today's inline answers.
    Reply { chat_id: String, text: String },
    /// The pairing request `access::enforce_dm_access` used to raise itself.
    RequestPairing {
        chat_id: String,
        from_id: i64,
        meta: serde_json::Value,
    },
}

/// Turn a raw Telegram Bot API Update into an [`InboundMessage`].
///
/// Handles: message extraction (message or channel_post), reply threading,
/// user identity resolution, and bot mention stripping. Deduplication is
/// not done here — it happens durably when the caller admits the parsed
/// message, keyed on the same `update_id`.
///
/// A re-delivered update is therefore normalized again, and its effects are
/// applied again with it. The five-minute cache that used to suppress those
/// also sat in front of admission, where it would have answered a re-delivery
/// of *unaccepted* work with a 200 and nothing durable behind it.
///
/// The update arrives already decoded: ingress needs the envelope to route on
/// before it gets here, and decoding the same bytes twice was one of the things
/// the split was supposed to end.
pub(super) async fn normalize(
    payload: &serde_json::Value,
    ctx: &NormalizeContext<'_>,
) -> Result<Normalization, AdapterError> {
    // 1. Extract update_id
    let update_id = payload
        .get("update_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing update_id".into()))?;

    // 2. Extract message (message or channel_post)
    let message = payload
        .get("message")
        .or_else(|| payload.get("channel_post"));

    let Some(message) = message else {
        debug!(
            update_id,
            "Telegram update has no message or channel_post, ignoring"
        );
        return Ok(ignored("no_message"));
    };

    // 4. Extract fields
    let chat_id = message
        .pointer("/chat/id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing chat.id".into()))?;

    let message_id = message
        .get("message_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing message_id".into()))?;

    let text = message.get("text").and_then(|v| v.as_str()).unwrap_or("");

    // 5. Extract sender (from field — absent in channel_post)
    let from = message.get("from");
    let from_id = from
        .and_then(|f| f.get("id"))
        .and_then(serde_json::Value::as_i64);

    let Some(from_id) = from_id else {
        debug!(update_id, "Telegram message has no from.id, ignoring");
        return Ok(ignored("no_sender"));
    };

    // 5.6. Handle /start and /help before access control (always respond)
    let chat_id_str = chat_id.to_string();
    if !text.is_empty() {
        let trimmed = text.trim();
        if trimmed == "/start" || trimmed.starts_with("/start@") {
            return Ok(answered(
                "start_command",
                &chat_id_str,
                "Welcome! Send me a message to get started.",
            ));
        }
        if trimmed == "/help" || trimmed.starts_with("/help@") {
            return Ok(answered(
                "help_command",
                &chat_id_str,
                "Send any message and I'll create a task for it.",
            ));
        }
    }

    if text.is_empty() {
        debug!(update_id, "Telegram message has no text, ignoring");
        return Ok(ignored("empty_text"));
    }

    // 6. Handle reply threading
    let reply_to_message_id = message
        .pointer("/reply_to_message/message_id")
        .and_then(serde_json::Value::as_i64);

    // 7. Strip bot mention from group messages
    let bot_username = ctx.tg_config.bot_username.as_deref();
    let clean_text = strip_bot_mention(text, bot_username);
    if clean_text.trim().is_empty() {
        return Ok(ignored("empty_after_mention"));
    }

    // 7.5. Access control
    let from_username = from
        .and_then(|f| f.get("username"))
        .and_then(|v| v.as_str());
    let from_first_name = from
        .and_then(|f| f.get("first_name"))
        .and_then(|v| v.as_str());
    let from_last_name = from
        .and_then(|f| f.get("last_name"))
        .and_then(|v| v.as_str());

    if ctx.is_private_chat {
        let decision = access::enforce_dm_access(&DmSender {
            tg_config: ctx.tg_config,
            from_id,
            from_username,
            from_first_name,
            from_last_name,
        });
        match decision {
            DmDecision::Allowed => {}
            DmDecision::Blocked => {
                debug!(from_id, chat_id, "Access denied");
                return Ok(ignored("dm_access_policy"));
            }
            DmDecision::RequestPairing { meta } => {
                debug!(from_id, chat_id, "Access denied, pairing requested");
                return Ok(Normalization::Ignored {
                    reason: "pairing_required",
                    effects: vec![Effect::RequestPairing {
                        chat_id: chat_id_str,
                        from_id,
                        meta,
                    }],
                });
            }
        }
    } else {
        let decision = access::enforce_group_access(ctx.tg_config, chat_id, &clean_text);
        match decision {
            AccessDecision::Blocked | AccessDecision::PairingSent => {
                debug!(from_id, chat_id, ?decision, "Access denied");
                return Ok(ignored("group_access_policy"));
            }
            AccessDecision::Allowed => {}
        }
    }

    // 8. Resolve Telegram user → system user (fall back to config owner for unmapped users)
    let from_id_str = from_id.to_string();
    let user_id = if let Some(uid) = super::auth::resolve_sender(&from_id_str, ctx.users).await? {
        uid
    } else {
        info!(
            telegram_user_id = from_id,
            chat_id,
            fallback_user = %ctx.config.effective_user_id(),
            "Telegram user not mapped, using config owner as fallback"
        );
        ctx.config.effective_user_id().to_string()
    };

    // 9. Extract forum/topic fields
    let is_forum = message
        .pointer("/chat/is_forum")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let forum_topic_id = message
        .get("message_thread_id")
        .and_then(serde_json::Value::as_i64);

    // 10. Build thread properties
    let thread_properties = serde_json::json!({
        "chat_id": chat_id_str,
        "message_id": message_id,
        "forum_topic_id": forum_topic_id,
    });

    // Threading strategy:
    // - Private chats (DMs): use chat_id as thread key so all messages from
    //   the same user thread together into one persistent task.
    // - Forum groups: use chat_id:topic_id as thread key.
    // - Non-forum groups: use reply_to_message_id for thread continuity,
    //   otherwise each message starts its own thread.
    let (thread_id, msg_thread_id, effective_source_id) = if ctx.is_private_chat {
        (
            Some(chat_id_str.clone()),
            chat_id_str.clone(),
            // Use wildcard source_id so router finds the wildcard config
            if ctx.config.source_id == "*" {
                "*".to_string()
            } else {
                chat_id_str.clone()
            },
        )
    } else if let (true, Some(topic_id)) = (is_forum, forum_topic_id) {
        let topic_key = format!("{chat_id_str}:{topic_id}");
        (Some(topic_key.clone()), topic_key, chat_id_str.clone())
    } else {
        let thread_id = reply_to_message_id.map(|id| id.to_string());
        let msg_thread_id = reply_to_message_id.unwrap_or(message_id).to_string();
        (thread_id, msg_thread_id, chat_id_str.clone())
    };

    info!(
        update_id,
        chat_id,
        message_id,
        from_id,
        thread_id = ?thread_id,
        is_private_chat = ctx.is_private_chat,
        is_forum,
        forum_topic_id,
        "Telegram message parsed"
    );

    // Telegram's `@username` where the sender has one — many accounts do not —
    // else the display name they do have.
    let author = from_username
        .map(|handle| format!("@{handle}"))
        .or_else(|| from_first_name.map(str::to_string))
        .and_then(|handle| moatless_loops::adapters::models::author_handle(&handle));

    Ok(Normalization::Message {
        message: Box::new(InboundMessage {
            adapter_kind: AdapterKind::Telegram,
            source_id: effective_source_id,
            thread_id,
            message_thread_id: msg_thread_id,
            user_id,
            author,
            sender_external_id: None,
            text: clean_text,
            file_ids: vec![],
            thread_properties,
            adapter_context: serde_json::json!({}),
            external_event_id: Some(super::external_id::message(message_id)),
            source_task_id: None,
            // Telegram makes no audience distinction: every event goes to every
            // task bound to the chat or topic, as it always has.
            audience: Audience::Everyone,
        }),
        effects: vec![],
    })
}

/// Nothing to route and nothing to do about it.
const fn ignored(reason: &'static str) -> Normalization {
    Normalization::Ignored {
        reason,
        effects: Vec::new(),
    }
}

/// A command the bot answers inline. The update itself still routes nowhere.
fn answered(reason: &'static str, chat_id: &str, text: &str) -> Normalization {
    Normalization::Ignored {
        reason,
        effects: vec![Effect::Reply {
            chat_id: chat_id.to_string(),
            text: text.to_string(),
        }],
    }
}

/// Strip bot mention from message text (e.g., "@mybot fix the bug" → "fix the bug").
fn strip_bot_mention(text: &str, bot_username: Option<&str>) -> String {
    let Some(username) = bot_username else {
        return text.to_string();
    };

    // Normalize: ensure username starts with @
    let mention = if username.starts_with('@') {
        username.to_string()
    } else {
        format!("@{username}")
    };

    text.replace(&mention, "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::strip_bot_mention;

    #[test]
    fn strip_mention_with_at() {
        assert_eq!(
            strip_bot_mention("@mybot fix the bug", Some("@mybot")),
            "fix the bug"
        );
    }

    #[test]
    fn strip_mention_without_at_prefix() {
        assert_eq!(
            strip_bot_mention("@mybot fix the bug", Some("mybot")),
            "fix the bug"
        );
    }

    #[test]
    fn strip_mention_no_bot_username() {
        assert_eq!(
            strip_bot_mention("@mybot fix the bug", None),
            "@mybot fix the bug"
        );
    }

    #[test]
    fn strip_mention_not_present() {
        assert_eq!(
            strip_bot_mention("fix the bug", Some("@mybot")),
            "fix the bug"
        );
    }
}
