//! Telegram adapter — Adapter trait impl + webhook parsing.

use moka::future::Cache;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AccessDecision, AdapterConnection, BindingContext, InboundMessage, NewThreadInfo,
    OutboundReply, TaskBinding, TaskTriggerRoute, TriggerContext,
};
use crate::adapters::{Adapter, AdapterKind, BindingDestination};
use crate::db::DbPool;

use super::access;
use super::api::TelegramApi;
use super::config::TelegramAdapterConfig;

/// Result of parsing a Telegram webhook update.
pub enum TelegramWebhookResult {
    /// Parsed message ready for routing.
    Message(Box<InboundMessage>),
    /// Update should be ignored (unsupported type, duplicate, etc.).
    Ignore,
}

/// Telegram adapter.
pub struct TelegramAdapter {
    api: TelegramApi,
    /// Deduplication cache: update_id → () with 5-minute TTL.
    dedup_cache: Cache<i64, ()>,
}

impl TelegramAdapter {
    pub fn new(bot_token: &str) -> Self {
        Self {
            api: TelegramApi::new(bot_token),
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_mins(5))
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
        config: &TriggerContext,
        is_private_chat: bool,
        db: &DbPool,
    ) -> Result<TelegramWebhookResult, AdapterError> {
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| AdapterError::BadRequest(format!("Invalid JSON: {e}")))?;

        // 1. Extract update_id
        let update_id = payload
            .get("update_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| AdapterError::BadRequest("Missing update_id".into()))?;

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
            debug!(
                update_id,
                "Telegram update has no message or channel_post, ignoring"
            );
            return Ok(TelegramWebhookResult::Ignore);
        };

        // 4. Extract fields
        let chat_id = message
            .pointer("/chat/id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| AdapterError::BadRequest("Missing chat.id".into()))?;

        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| AdapterError::BadRequest("Missing message_id".into()))?;

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
        let tg_config = TelegramAdapterConfig::from_config(config);

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
        let from_username = from
            .and_then(|f| f.get("username"))
            .and_then(|v| v.as_str());
        let from_first_name = from
            .and_then(|f| f.get("first_name"))
            .and_then(|v| v.as_str());
        let from_last_name = from
            .and_then(|f| f.get("last_name"))
            .and_then(|v| v.as_str());

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
                fallback_user = %config.effective_user_id(),
                "Telegram user not mapped, using config owner as fallback"
            );
            config.effective_user_id().to_string()
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
            is_private_chat,
            is_forum,
            forum_topic_id,
            "Telegram message parsed"
        );

        Ok(TelegramWebhookResult::Message(Box::new(InboundMessage {
            provider: AdapterKind::Telegram,
            source_id: effective_source_id,
            thread_id,
            message_thread_id: msg_thread_id,
            user_id,
            text: clean_text,
            file_ids: vec![],
            thread_properties,
            provider_context: serde_json::json!({}),
            external_event_id: Some(super::external_id::message(message_id)),
            source_task_id: None,
        })))
    }
}

#[async_trait::async_trait]
impl Adapter for TelegramAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Telegram
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn binding_destination_from_existing_thread(
        &self,
        source_id: &str,
        external_thread_id: &str,
        thread_properties: &serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, AdapterError> {
        let chat_id = thread_properties
            .get("chat_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(source_id);
        let forum_topic_id = thread_properties
            .get("forum_topic_id")
            .and_then(serde_json::Value::as_i64);
        let subject_key = match forum_topic_id {
            Some(topic_id) => format!("{chat_id}:topic:{topic_id}"),
            None => format!("{chat_id}:thread:{external_thread_id}"),
        };
        let mut destination_config = serde_json::json!({
            "chat_id": chat_id,
            "message_id": thread_properties
                .get("message_id")
                .and_then(serde_json::Value::as_i64),
            "forum_topic_id": forum_topic_id,
        });
        if let Some(filter_user_id) = filter_user_id {
            destination_config["filter_user_id"] = serde_json::json!(filter_user_id);
        }

        Ok(BindingDestination::new(
            "telegram_thread",
            subject_key,
            destination_config,
        ))
    }

    fn binding_destination_from_new_thread(
        &self,
        rule: &TaskTriggerRoute,
        new_thread_info: &NewThreadInfo,
    ) -> Result<BindingDestination, AdapterError> {
        let destination = self.binding_destination_from_existing_thread(
            &rule.source_matcher,
            &new_thread_info.external_thread_id,
            &new_thread_info.properties,
            None,
        )?;
        Ok(destination.with_last_external_event_id(new_thread_info.external_thread_id.clone()))
    }

    fn external_id_aliases(&self, external_id: &str) -> Vec<String> {
        super::external_id::aliases(external_id)
    }

    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, AdapterError> {
        // Look up by provider_user_id in provider_tokens table
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT user_id FROM provider_tokens WHERE provider = 'telegram' AND provider_user_id = $1 LIMIT 1",
        )
        .bind(identity)
        .fetch_optional(db)
        .await
        .map_err(|e| AdapterError::Internal(format!("User lookup failed: {e}")))?;

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
        thread: &BindingContext,
        _config: &TriggerContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let chat_id = thread
            .destination_config
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AdapterError::Internal("No chat_id in thread properties".into()))?;

        let reply_to = thread
            .destination_config
            .get("message_id")
            .and_then(serde_json::Value::as_i64);

        let forum_topic_id = thread
            .destination_config
            .get("forum_topic_id")
            .and_then(serde_json::Value::as_i64);

        let mut text = message.text.clone();
        if let Some(ref url) = message.task_url {
            use std::fmt::Write;
            let _ = write!(text, "\n\n[View task]({url})");
        }

        self.api
            .send_message(chat_id, &text, reply_to, forum_topic_id)
            .await?;
        Ok(None)
    }

    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        _connection: &AdapterConnection,
        rule: Option<&TaskTriggerRoute>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let thread = crate::bindings::runtime::binding_context(binding);
        let config = rule.map_or_else(
            || TriggerContext {
                id: binding.trigger_id.clone().unwrap_or_default(),
                provider: "telegram".to_string(),
                source_id: binding
                    .destination_config
                    .get("chat_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                source_name: None,
                repository_id: String::new(),
                routing_mode: crate::adapters::models::RoutingMode::BySubject,
                prompt_template: None,
                active: true,
                allow_direct_send: false,
                provider_config: None,
                ongoing_task_id: None,
                scope: moatless_backend_core::scope::Scope::Global,
                team_id: None,
                task_name: None,
                tag_ids: vec![],
                run_as_user_id: None,
                agent_type: "claude-code".to_string(),
                inactive_ttl_days: None,
                created_by: "system".to_string(),
                created_at: binding.created_at,
                updated_at: binding.last_event_at,
            },
            |r| crate::bindings::runtime::trigger_context(r, "telegram"),
        );
        self.send_reply(&thread, &config, message).await
    }

    async fn send_message(
        &self,
        config: &TriggerContext,
        message: &OutboundReply,
    ) -> Result<NewThreadInfo, AdapterError> {
        let chat_id = &config.source_id;

        let mut text = message.text.clone();
        if let Some(ref url) = message.task_url {
            use std::fmt::Write;
            let _ = write!(text, "\n\n[View task]({url})");
        }

        let message_id = self
            .api
            .send_message_returning_id(chat_id, &text, None, None)
            .await?;
        Ok(NewThreadInfo {
            external_thread_id: message_id.to_string(),
            properties: serde_json::json!({
                "chat_id": chat_id,
                "message_id": message_id,
                "forum_topic_id": serde_json::Value::Null,
            }),
        })
    }

    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        _connection: &AdapterConnection,
        _rule: Option<&TaskTriggerRoute>,
        target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let target = super::external_id::message_id_from(target_external_id).ok_or_else(|| {
            AdapterError::BadRequest(format!(
                "Telegram reply target `{target_external_id}` is not a tg:m:<id> id"
            ))
        })?;
        let chat_id = binding
            .destination_config
            .get("chat_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::BadRequest(
                    "Telegram binding destination_config has no `chat_id` field".into(),
                )
            })?
            .to_string();
        let forum_topic_id = binding
            .destination_config
            .get("forum_topic_id")
            .and_then(serde_json::Value::as_i64);

        let mut text = message.text.clone();
        if let Some(ref url) = message.task_url {
            use std::fmt::Write;
            let _ = write!(text, "\n\n[View task]({url})");
        }

        let new_id = self
            .api
            .send_message_returning_id(&chat_id, &text, Some(target), forum_topic_id)
            .await?;
        Ok(Some(super::external_id::message(new_id)))
    }

    async fn add_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        let (chat_id, message_id) = parse_telegram_reaction_target(binding, target_external_id)?;
        let trimmed = emoji.trim();
        if trimmed.is_empty() {
            return Err(AdapterError::BadRequest("Empty emoji".into()));
        }
        self.api
            .set_message_reaction(&chat_id, message_id, Some(trimmed))
            .await?;
        Ok(None)
    }

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        _emoji: &str,
    ) -> Result<(), AdapterError> {
        // Telegram's API is an idempotent set: clearing means setting an empty list.
        // The `_emoji` argument is accepted for parity with other providers but
        // ignored — Telegram replaces the bot's full reaction list either way.
        let (chat_id, message_id) = parse_telegram_reaction_target(binding, target_external_id)?;
        self.api
            .set_message_reaction(&chat_id, message_id, None)
            .await
    }

    // fetch_context: default no-op (Bot API doesn't support fetching chat history)
}

/// Resolve the `(chat_id, message_id)` pair for a Telegram reaction call.
///
/// `target_external_id` carries the message id (`tg:m:<id>`); `chat_id` comes
/// from the binding's `destination_config`.
fn parse_telegram_reaction_target(
    binding: &BindingContext,
    target_external_id: &str,
) -> Result<(String, i64), AdapterError> {
    let message_id = super::external_id::message_id_from(target_external_id).ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "Telegram reaction target `{target_external_id}` is not a tg:m:<id> id"
        ))
    })?;
    let chat_id = binding
        .destination_config
        .get("chat_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            AdapterError::BadRequest(
                "Telegram binding destination_config has no `chat_id` field".into(),
            )
        })?
        .to_string();
    Ok((chat_id, message_id))
}

/// Resolve the trigger source for a raw Telegram update and route it through
/// `parse_update` → `route_inbound_message`. Shared by the webhook handler and the
/// polling loop so the config-lookup + routing logic is not duplicated.
pub async fn resolve_and_process_telegram_update(
    state: &crate::state::AppState,
    telegram_provider: &TelegramAdapter,
    body: &[u8],
) -> Result<(), AdapterError> {
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| AdapterError::BadRequest(format!("Invalid JSON: {e}")))?;

    // Extract chat_id from message or channel_post
    let message = payload
        .get("message")
        .or_else(|| payload.get("channel_post"));
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

    // Look up rules — exact match first, then wildcard. Telegram uses a single
    // env-backed bot transport, so the connection scope is "default".
    let Some((connection, rule)) = crate::bindings::runtime::resolve_chat_provider_event(
        state, "telegram", "default", &source_id,
    )
    .await?
    else {
        info!(
            source_id,
            is_private_chat, "Telegram message ignored: no task trigger for this chat"
        );
        return Ok(());
    };
    let config = crate::bindings::runtime::trigger_context(&rule, "telegram");

    let result = telegram_provider
        .parse_update(body, &config, is_private_chat, &state.db)
        .await?;

    match result {
        TelegramWebhookResult::Message(msg) => {
            let msg = *msg;
            info!(
                source_id = %msg.source_id,
                thread_id = ?msg.thread_id,
                user_id = %msg.user_id,
                "Telegram message routed for processing"
            );
            let destination = telegram_provider.binding_destination_from_existing_thread(
                &msg.source_id,
                &msg.message_thread_id,
                &msg.thread_properties,
                None,
            )?;
            let state_clone = state.clone();
            let connection = connection.clone();
            let rule = rule.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    crate::triggers::delivery::AdapterEventDeliveryService::new(&state_clone)
                        .deliver(
                            &connection,
                            &rule,
                            msg,
                            "telegram",
                            destination.subject_kind,
                            &destination.subject_key,
                            destination.destination_config,
                        )
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
    use crate::adapters::models::{RoutingMode, TaskTriggerRoute};
    use crate::adapters::Adapter;
    use moatless_backend_core::scope::Scope;

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
        let provider = TelegramAdapter::new("fake_token");

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
        let provider = TelegramAdapter::new("fake_token");

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
        let _ = provider
            .parse_update(&body, &config, false, &fake_db_pool())
            .await;

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
        let provider = TelegramAdapter::new("fake_token");

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
        let provider = TelegramAdapter::new("fake_token");

        let update = serde_json::json!({
            "message": { "message_id": 1, "chat": { "id": 1 }, "text": "hi" }
        });

        let body = serde_json::to_vec(&update).unwrap();
        let config = test_config();
        let result = provider
            .parse_update(&body, &config, false, &fake_db_pool())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn parse_no_message_ignored() {
        let provider = TelegramAdapter::new("fake_token");

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
    fn binding_destination_from_new_thread_preserves_direct_send_shape() {
        let provider = TelegramAdapter::new("fake_token");
        let rule = test_rule("telegram", "111");
        let destination = provider
            .binding_destination_from_new_thread(
                &rule,
                &NewThreadInfo {
                    external_thread_id: "77".to_string(),
                    properties: serde_json::json!({
                        "chat_id": "111",
                        "message_id": 77,
                        "forum_topic_id": serde_json::Value::Null,
                    }),
                },
            )
            .unwrap();

        assert_eq!(destination.subject_kind, "telegram_thread");
        assert_eq!(destination.subject_key, "111:thread:77");
        assert_eq!(
            destination.destination_config,
            serde_json::json!({
                "chat_id": "111",
                "message_id": 77,
                "forum_topic_id": serde_json::Value::Null,
            })
        );
        assert_eq!(destination.last_external_event_id.as_deref(), Some("77"));
    }

    fn test_config() -> TriggerContext {
        TriggerContext {
            id: "test".into(),
            provider: "telegram".into(),
            source_id: "111".into(),
            source_name: None,
            repository_id: "repo1".into(),
            routing_mode: crate::adapters::models::RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            provider_config: None,
            ongoing_task_id: None,
            scope: moatless_backend_core::scope::Scope::User,
            team_id: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            inactive_ttl_days: None,
            created_by: "system".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn test_rule(provider: &str, source_matcher: &str) -> TaskTriggerRoute {
        let now = chrono::Utc::now();
        TaskTriggerRoute {
            id: "tt_test".into(),
            provider: provider.into(),
            connection_id: "pc_test".into(),
            source_matcher: source_matcher.into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repo1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            inactive_ttl_days: None,
            allow_direct_send: true,
            provider_config: None,
            ongoing_task_id: None,
            close_task_on_close: true,
            notify_on_success: false,
            active: true,
            created_by: "system".into(),
            scope: Scope::Global,
            team_id: None,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
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
