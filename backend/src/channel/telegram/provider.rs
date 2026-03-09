//! Telegram channel provider — ChannelProvider trait impl + webhook parsing.

use moka::future::Cache;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::channel::error::ChannelError;
use crate::channel::models::{
    AccessDecision, ChannelAccessPolicy, ChannelConfig, ChannelThread, InboundMessage,
    MessageStatus, OutboundReply,
};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::db::DbPool;

use super::access;
use super::api::TelegramApi;
use super::config::TelegramProviderConfig;

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

        // 5. Extract sender (from field — absent in channel_post)
        let from = message.get("from");
        let from_id = from
            .and_then(|f| f.get("id"))
            .and_then(serde_json::Value::as_i64);

        let Some(from_id) = from_id else {
            debug!(update_id, "Telegram message has no from.id, ignoring");
            return Ok(TelegramWebhookResult::Ignore);
        };

        // 5.5. Parse typed provider config
        let tg_config = TelegramProviderConfig::from_config(config);

        // 5.6. Handle /start and /help before access control (always respond)
        let chat_id_str = chat_id.to_string();
        if !text.is_empty() {
            let trimmed = text.trim();
            if trimmed == "/start" || trimmed.starts_with("/start@") {
                self.api
                    .send_message(
                        &chat_id_str,
                        "Welcome! Send me a message to get started.",
                        None,
                        None,
                    )
                    .await
                    .ok();
                return Ok(TelegramWebhookResult::Ignore);
            }
            if trimmed == "/help" || trimmed.starts_with("/help@") {
                self.api
                    .send_message(
                        &chat_id_str,
                        "Send any message and I'll create a task for it.",
                        None,
                        None,
                    )
                    .await
                    .ok();
                return Ok(TelegramWebhookResult::Ignore);
            }
        }

        if text.is_empty() {
            debug!(update_id, "Telegram message has no text, ignoring");
            return Ok(TelegramWebhookResult::Ignore);
        }

        // 6. Handle reply threading
        let reply_to_message_id = message
            .pointer("/reply_to_message/message_id")
            .and_then(serde_json::Value::as_i64);

        // 7. Strip bot mention from group messages
        let bot_username = tg_config.bot_username.as_deref();
        let clean_text = strip_bot_mention(text, bot_username);
        if clean_text.trim().is_empty() {
            return Ok(TelegramWebhookResult::Ignore);
        }

        // 7.5. Access control
        let from_username = from.and_then(|f| f.get("username")).and_then(|v| v.as_str());
        let from_first_name = from.and_then(|f| f.get("first_name")).and_then(|v| v.as_str());
        let from_last_name = from.and_then(|f| f.get("last_name")).and_then(|v| v.as_str());

        let access_decision = if is_private_chat {
            let ctx = access::DmAccessContext {
                tg_config: &tg_config,
                from_id,
                from_username,
                from_first_name,
                from_last_name,
                config,
                api: &self.api,
                chat_id,
                db,
            };
            access::enforce_dm_access(&ctx).await?
        } else {
            access::enforce_group_access(&tg_config, chat_id, &clean_text)
        };

        match access_decision {
            AccessDecision::Blocked | AccessDecision::PairingSent => {
                debug!(from_id, chat_id, ?access_decision, "Access denied");
                return Ok(TelegramWebhookResult::Ignore);
            }
            AccessDecision::Allowed => {}
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
        let (thread_id, msg_thread_id, effective_source_id) = if is_private_chat {
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
        } else if let (true, Some(topic_id)) = (is_forum, forum_topic_id) {
            let topic_key = format!("{chat_id_str}:{topic_id}");
            (
                Some(topic_key.clone()),
                topic_key,
                chat_id_str.clone(),
            )
        } else {
            let thread_id = reply_to_message_id.map(|id| id.to_string());
            let msg_thread_id = reply_to_message_id
                .unwrap_or(message_id)
                .to_string();
            (thread_id, msg_thread_id, chat_id_str.clone())
        };

        info!(
            update_id,
            chat_id,
            message_id,
            from_id,
            thread_id = ?thread_id,
            is_private_chat,
            is_forum,
            forum_topic_id,
            "Telegram message parsed"
        );

        Ok(TelegramWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::Telegram,
            source_id: effective_source_id,
            thread_id,
            message_thread_id: msg_thread_id,
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

        let forum_topic_id = thread
            .properties
            .get("forum_topic_id")
            .and_then(serde_json::Value::as_i64);

        let mut text = message.text.clone();
        if let Some(ref url) = message.task_url {
            use std::fmt::Write;
            let _ = write!(text, "\n\n[View task]({url})");
        }

        self.api
            .send_message(chat_id, &text, reply_to, forum_topic_id)
            .await
    }

    async fn set_status(
        &self,
        thread: &ChannelThread,
        _config: &ChannelConfig,
        status: MessageStatus,
    ) -> Result<(), ChannelError> {
        let chat_id = match thread.properties.get("chat_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return Ok(()),
        };

        // Send typing indicator
        if matches!(status, MessageStatus::Processing) {
            let _ = self.api.send_chat_action(chat_id, "typing").await;
        }

        // Set reaction emoji (best-effort)
        if let Some(message_id) = thread.properties.get("message_id").and_then(serde_json::Value::as_i64) {
            let emoji = match status {
                MessageStatus::Processing => "\u{1F440}",   // 👀
                MessageStatus::Completed => "\u{2705}",     // ✅
                MessageStatus::Error => "\u{274C}",         // ❌
                MessageStatus::PendingResponse => return Ok(()),
            };
            let _ = self
                .api
                .set_message_reaction(chat_id, message_id, emoji)
                .await;
        }

        Ok(())
    }

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

    // Extract from_id early so auto_create_config can resolve the sender's
    // default repository before parse_update runs.
    let from_id = message
        .and_then(|m| m.pointer("/from/id"))
        .and_then(serde_json::Value::as_i64);

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
                    match auto_create_config(state, &source_id, message, is_private_chat, from_id).await {
                        Ok(c) => c,
                        Err(e) => {
                            warn!(source_id, error = %e, "Failed to auto-create Telegram config");
                            return Ok(());
                        }
                    }
                }
            }
        }
        None => {
            match auto_create_config(state, &source_id, message, is_private_chat, from_id).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(source_id, error = %e, "Failed to auto-create Telegram config");
                    return Ok(());
                }
            }
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

/// Auto-create a channel config for an unknown Telegram chat.
///
/// Uses the shared wildcard config helper to clone from a `source_id="*"` template.
/// Falls back to the seeded "Moatable" template repository if no wildcard exists.
async fn auto_create_config(
    state: &crate::state::AppState,
    source_id: &str,
    message: Option<&serde_json::Value>,
    is_private_chat: bool,
    from_id: Option<i64>,
) -> Result<ChannelConfig, ChannelError> {
    use crate::channel::models::RoutingMode;
    use crate::scope::Scope;

    // Try to resolve sender → system user (for ownership of the created config)
    let owner = resolve_sender_user(state, from_id).await;

    let chat_name = message
        .and_then(|m| m.pointer("/chat/title"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            message
                .and_then(|m| m.pointer("/chat/first_name"))
                .and_then(|v| v.as_str())
        })
        .map(String::from);

    // Try wildcard config clone first
    let team_id = None; // Telegram doesn't have team context at webhook time
    let created_by = owner.as_deref().unwrap_or("system");
    if let Some(config) = crate::channel::defaults::create_config_from_wildcard(
        &*state.channel_configs,
        "telegram",
        source_id,
        chat_name.clone(),
        team_id,
        created_by,
    )
    .await?
    {
        info!(
            config_id = %config.id,
            source_id,
            "Auto-created Telegram config from wildcard template"
        );
        return Ok(config);
    }

    // Final fallback: create from Moatable template
    let repo = state
        .repositories
        .find_by_name_and_user("Moatable", "system")
        .await?
        .ok_or_else(|| {
            ChannelError::Internal("Default Moatable repository not found".into())
        })?;

    let now = chrono::Utc::now();

    // DMs use Pairing policy so the owner must approve new users;
    // groups default to Open.
    let provider_config = if is_private_chat {
        Some(serde_json::to_value(TelegramProviderConfig {
            dm_access_policy: ChannelAccessPolicy::Pairing,
            ..Default::default()
        }).unwrap_or_default())
    } else {
        None
    };

    let new_config = ChannelConfig {
        id: nanoid::nanoid!(21),
        provider: "telegram".to_string(),
        source_id: source_id.to_string(),
        source_name: chat_name,
        repository_id: repo.id.clone(),
        routing_mode: RoutingMode::Ongoing,
        prompt_template: None,
        active: true,
        provider_config,
        ongoing_task_id: None,
        response_config_id: None,
        scope: Scope::Global,
        team_id: None,
        created_by: owner.unwrap_or_else(|| "system".to_string()),
        created_at: now,
        updated_at: now,
    };

    state.channel_configs.insert(&new_config).await?;
    info!(
        config_id = %new_config.id,
        source_id,
        repository = %repo.name,
        "Auto-created Telegram channel config (Moatable fallback)"
    );
    Ok(new_config)
}

/// Resolve a Telegram `from_id` → system user ID (for ownership).
async fn resolve_sender_user(
    state: &crate::state::AppState,
    from_id: Option<i64>,
) -> Option<String> {
    let telegram_id = from_id?;
    let from_id_str = telegram_id.to_string();

    sqlx::query_scalar(
        "SELECT user_id FROM provider_tokens WHERE provider = 'telegram' AND provider_user_id = $1 LIMIT 1",
    )
    .bind(&from_id_str)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
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
            scope: crate::scope::Scope::User,
            team_id: None,
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
