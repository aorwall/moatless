//! Telegram channel provider — ChannelProvider trait impl + webhook parsing.

use moka::future::Cache;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::channel::error::ChannelError;
use crate::channel::models::{ChannelConfig, ChannelThread, InboundMessage, OutboundReply};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::db::DbPool;

use super::api::TelegramApi;

/// Result of parsing a Telegram webhook update.
pub enum TelegramWebhookResult {
    /// Parsed message ready for routing.
    Message(Box<InboundMessage>),
    /// Update should be ignored (unsupported type, duplicate, etc.).
    Ignore,
}

/// Telegram channel provider.
pub struct TelegramProvider {
    api: TelegramApi,
    /// Deduplication cache: update_id → () with 5-minute TTL.
    dedup_cache: Cache<i64, ()>,
}

impl TelegramProvider {
    pub fn new(bot_token: &str) -> Self {
        Self {
            api: TelegramApi::new(bot_token),
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_secs(300))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Access the underlying Telegram API client (used by the poller).
    pub fn api(&self) -> &TelegramApi {
        &self.api
    }

    /// Parse a Telegram Bot API Update into an InboundMessage.
    ///
    /// Handles: deduplication, message extraction (message or channel_post),
    /// reply threading, user identity resolution, and bot mention stripping.
    pub async fn parse_update(
        &self,
        body: &[u8],
        config: &ChannelConfig,
        is_private_chat: bool,
        db: &DbPool,
    ) -> Result<TelegramWebhookResult, ChannelError> {
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

        // 1. Extract update_id
        let update_id = payload
            .get("update_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| ChannelError::BadRequest("Missing update_id".into()))?;

        // 2. Deduplicate
        if self.dedup_cache.get(&update_id).await.is_some() {
            debug!(update_id, "Duplicate Telegram update, ignoring");
            return Ok(TelegramWebhookResult::Ignore);
        }
        self.dedup_cache.insert(update_id, ()).await;

        // 3. Extract message (message or channel_post)
        let message = payload
            .get("message")
            .or_else(|| payload.get("channel_post"));

        let Some(message) = message else {
            debug!(update_id, "Telegram update has no message or channel_post, ignoring");
            return Ok(TelegramWebhookResult::Ignore);
        };

        // 4. Extract fields
        let chat_id = message
            .pointer("/chat/id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| ChannelError::BadRequest("Missing chat.id".into()))?;

        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| ChannelError::BadRequest("Missing message_id".into()))?;

        let text = message.get("text").and_then(|v| v.as_str()).unwrap_or("");

        if text.is_empty() {
            debug!(update_id, "Telegram message has no text, ignoring");
            return Ok(TelegramWebhookResult::Ignore);
        }

        // 5. Extract sender (from field — absent in channel_post)
        let from = message.get("from");
        let from_id = from
            .and_then(|f| f.get("id"))
            .and_then(serde_json::Value::as_i64);

        let Some(from_id) = from_id else {
            debug!(update_id, "Telegram message has no from.id, ignoring");
            return Ok(TelegramWebhookResult::Ignore);
        };

        // 6. Handle reply threading
        let reply_to_message_id = message
            .pointer("/reply_to_message/message_id")
            .and_then(serde_json::Value::as_i64);

        // 7. Strip bot mention from group messages
        let bot_username = config
            .provider_config
            .as_ref()
            .and_then(|c| c.get("bot_username"))
            .and_then(|v| v.as_str());

        let clean_text = strip_bot_mention(text, bot_username);
        if clean_text.trim().is_empty() {
            return Ok(TelegramWebhookResult::Ignore);
        }

        // 8. Resolve Telegram user → system user (fall back to config owner for unmapped users)
        let from_id_str = from_id.to_string();
        let user_id = if let Some(uid) = self.resolve_user(&from_id_str, db).await? {
            uid
        } else {
            info!(
                telegram_user_id = from_id,
                chat_id,
                fallback_user = %config.created_by,
                "Telegram user not mapped, using config owner as fallback"
            );
            config.created_by.clone()
        };

        // 9. Build thread properties
        let chat_id_str = chat_id.to_string();
        let thread_properties = serde_json::json!({
            "chat_id": chat_id_str,
            "message_id": message_id,
        });

        // Threading strategy:
        // - Private chats (DMs): use chat_id as thread key so all messages from
        //   the same user thread together into one persistent task.
        // - Group chats: use reply_to_message_id for thread continuity,
        //   otherwise each message starts its own thread.
        let (thread_id, message_thread_id, effective_source_id) = if is_private_chat {
            (
                Some(chat_id_str.clone()),
                chat_id_str.clone(),
                // Use wildcard source_id so router finds the wildcard config
                if config.source_id == "*" {
                    "*".to_string()
                } else {
                    chat_id_str.clone()
                },
            )
        } else {
            let thread_id = reply_to_message_id.map(|id| id.to_string());
            let message_thread_id = reply_to_message_id
                .unwrap_or(message_id)
                .to_string();
            (thread_id, message_thread_id, chat_id_str.clone())
        };

        info!(
            update_id,
            chat_id,
            message_id,
            from_id,
            thread_id = ?thread_id,
            is_private_chat,
            "Telegram message parsed"
        );

        Ok(TelegramWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::Telegram,
            source_id: effective_source_id,
            thread_id,
            message_thread_id,
            user_id,
            text: clean_text,
            file_ids: vec![],
            thread_properties,
            provider_context: serde_json::json!({}),
        })))
    }
}

#[async_trait::async_trait]
impl ChannelProvider for TelegramProvider {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        // Look up by provider_user_id in provider_tokens table
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT user_id FROM provider_tokens WHERE provider = 'telegram' AND provider_user_id = $1 LIMIT 1",
        )
        .bind(identity)
        .fetch_optional(db)
        .await
        .map_err(|e| ChannelError::Internal(format!("User lookup failed: {e}")))?;

        if let Some((user_id,)) = row {
            info!(telegram_user_id = %identity, user_id = %user_id, "Resolved Telegram user to system user");
            Ok(Some(user_id))
        } else {
            warn!(telegram_user_id = %identity, "No system user found for Telegram user");
            Ok(None)
        }
    }

    async fn send_reply(
        &self,
        thread: &ChannelThread,
        _config: &ChannelConfig,
        message: &OutboundReply,
    ) -> Result<(), ChannelError> {
        let chat_id = thread
            .properties
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Internal("No chat_id in thread properties".into()))?;

        let reply_to = thread
            .properties
            .get("message_id")
            .and_then(serde_json::Value::as_i64);

        let mut text = message.text.clone();
        if let Some(ref url) = message.task_url {
            use std::fmt::Write;
            let _ = write!(text, "\n\n[View task]({url})");
        }

        self.api.send_message(chat_id, &text, reply_to).await
    }

    // set_status: default no-op (no Telegram equivalent to Slack reactions)
    // fetch_context: default no-op (Bot API doesn't support fetching chat history)
}

/// Resolve the channel config for a raw Telegram update and route it through
/// `parse_update` → `process_inbound`. Shared by the webhook handler and the
/// polling loop so the config-lookup + routing logic is not duplicated.
pub async fn resolve_and_process_telegram_update(
    state: &crate::state::AppState,
    providers: &std::sync::Arc<crate::channel::provider::ChannelProviders>,
    telegram_provider: &TelegramProvider,
    body: &[u8],
) -> Result<(), ChannelError> {
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

    // Extract chat_id from message or channel_post
    let message = payload.get("message").or_else(|| payload.get("channel_post"));
    let chat_id = message
        .and_then(|m| m.pointer("/chat/id"))
        .and_then(serde_json::Value::as_i64);

    let Some(chat_id) = chat_id else {
        debug!("Telegram update has no chat.id, ignoring");
        return Ok(());
    };

    let source_id = chat_id.to_string();

    let chat_type = message
        .and_then(|m| m.pointer("/chat/type"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let is_private_chat = chat_type == "private";

    // Look up config — exact match first, then wildcard for DMs
    let config = state
        .channel_configs
        .find_by_provider_source("telegram", &source_id)
        .await?;

    let config = match config {
        Some(c) if c.active => c,
        Some(_) => {
            info!(source_id, "Telegram message ignored: channel config inactive");
            return Ok(());
        }
        None if is_private_chat => {
            match state
                .channel_configs
                .find_by_provider_source("telegram", "*")
                .await?
            {
                Some(c) if c.active => {
                    info!(source_id, "Telegram DM matched wildcard config");
                    c
                }
                _ => {
                    info!(source_id, "Telegram DM ignored: no active wildcard config");
                    return Ok(());
                }
            }
        }
        None => {
            info!(source_id, "Telegram message ignored: no channel config");
            return Ok(());
        }
    };

    let result = telegram_provider
        .parse_update(body, &config, is_private_chat, &state.db)
        .await?;

    match result {
        TelegramWebhookResult::Message(msg) => {
            info!(
                source_id = %msg.source_id,
                thread_id = ?msg.thread_id,
                user_id = %msg.user_id,
                "Telegram message routed for processing"
            );
            let state_clone = state.clone();
            let providers_clone = providers.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    crate::channel::inbound::process_inbound(&state_clone, &providers_clone, *msg)
                        .await
                {
                    warn!(error = %e, "Failed to process Telegram inbound message");
                }
            });
        }
        TelegramWebhookResult::Ignore => {
            debug!("Telegram update ignored by provider");
        }
    }

    Ok(())
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
    use super::*;

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

    #[tokio::test]
    async fn parse_no_text_ignored() {
        let provider = TelegramProvider::new("fake_token");

        let update = serde_json::json!({
            "update_id": 12346,
            "message": {
                "message_id": 43,
                "from": { "id": 999, "first_name": "Test" },
                "chat": { "id": 111, "type": "private" },
                "sticker": { "file_id": "abc" }
            }
        });

        let body = serde_json::to_vec(&update).unwrap();
        let config = test_config();
        let result = provider
            .parse_update(&body, &config, false, &fake_db_pool())
            .await;

        match result {
            Ok(TelegramWebhookResult::Ignore) => {}
            _ => panic!("Expected Ignore for message without text"),
        }
    }

    #[tokio::test]
    async fn parse_duplicate_update_ignored() {
        let provider = TelegramProvider::new("fake_token");

        let update = serde_json::json!({
            "update_id": 99999,
            "message": {
                "message_id": 44,
                "from": { "id": 999, "first_name": "Test" },
                "chat": { "id": 111, "type": "private" },
                "text": "hello"
            }
        });

        let body = serde_json::to_vec(&update).unwrap();
        let config = test_config();

        // First call reaches DB and fails (expected) — but populates dedup cache
        let _ = provider.parse_update(&body, &config, false, &fake_db_pool()).await;

        // Second call should be deduplicated before reaching DB
        let result = provider
            .parse_update(&body, &config, false, &fake_db_pool())
            .await;

        match result {
            Ok(TelegramWebhookResult::Ignore) => {}
            _ => panic!("Expected Ignore for duplicate update"),
        }
    }

    #[tokio::test]
    async fn parse_no_from_ignored() {
        let provider = TelegramProvider::new("fake_token");

        let update = serde_json::json!({
            "update_id": 12347,
            "channel_post": {
                "message_id": 45,
                "chat": { "id": -100123, "type": "channel" },
                "text": "channel message"
            }
        });

        let body = serde_json::to_vec(&update).unwrap();
        let config = test_config();
        let result = provider
            .parse_update(&body, &config, false, &fake_db_pool())
            .await;

        match result {
            Ok(TelegramWebhookResult::Ignore) => {}
            _ => panic!("Expected Ignore for channel post without from"),
        }
    }

    #[tokio::test]
    async fn parse_missing_update_id_fails() {
        let provider = TelegramProvider::new("fake_token");

        let update = serde_json::json!({
            "message": { "message_id": 1, "chat": { "id": 1 }, "text": "hi" }
        });

        let body = serde_json::to_vec(&update).unwrap();
        let config = test_config();
        let result = provider.parse_update(&body, &config, false, &fake_db_pool()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn parse_no_message_ignored() {
        let provider = TelegramProvider::new("fake_token");

        let update = serde_json::json!({
            "update_id": 12348,
            "inline_query": { "id": "abc", "query": "test" }
        });

        let body = serde_json::to_vec(&update).unwrap();
        let config = test_config();
        let result = provider
            .parse_update(&body, &config, false, &fake_db_pool())
            .await;

        match result {
            Ok(TelegramWebhookResult::Ignore) => {}
            _ => panic!("Expected Ignore for non-message update"),
        }
    }

    #[test]
    fn negative_chat_id_for_groups() {
        let chat_id: i64 = -1001234567890;
        let source_id = chat_id.to_string();
        assert_eq!(source_id, "-1001234567890");
    }

    fn test_config() -> ChannelConfig {
        ChannelConfig {
            id: "test".into(),
            provider: "telegram".into(),
            source_id: "111".into(),
            source_name: None,
            repository_id: "repo1".into(),
            routing_mode: crate::channel::models::RoutingMode::PerMessage,
            prompt_template: None,
            active: true,
            provider_config: None,
            ongoing_task_id: None,
            response_config_id: None,
            created_by: "system".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn fake_db_pool() -> DbPool {
        use sqlx::postgres::PgPoolOptions;
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://fake:fake@localhost:1/fake")
            .unwrap()
    }
}
