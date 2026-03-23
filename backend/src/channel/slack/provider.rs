//! Slack channel provider — ChannelProvider trait impl + webhook parsing.

use axum::http::HeaderMap;
use moka::future::Cache;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::channel::error::ChannelError;
use crate::channel::models::{AccessDecision, ChannelTrigger};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::channel::models::{InboundMessage, MessageStatus, NewThreadInfo, OutboundReply};
use crate::channel::models::{ChannelConfig, ChannelThread};
use crate::db::DbPool;

use super::access::enforce_slack_access;
use super::api::SlackApi;
use super::config::SlackProviderConfig;
use super::signature;

/// Result of parsing a Slack webhook event.
pub enum SlackWebhookResult {
    /// URL verification challenge — return the challenge string immediately.
    Challenge(String),
    /// Parsed message ready for routing.
    Message(InboundMessage),
    /// Event should be ignored (bot's own message, unsupported event, etc.).
    Ignore,
}

/// Slack channel provider.
pub struct SlackProvider {
    api: SlackApi,
    signing_secret: String,
    bot_user_id: Option<String>,
    /// Deduplication cache: event_id → () with 5-minute TTL.
    dedup_cache: Cache<String, ()>,
}

impl SlackProvider {
    pub fn new(bot_token: &str, signing_secret: &str, bot_user_id: Option<String>) -> Self {
        Self {
            api: SlackApi::new(bot_token),
            signing_secret: signing_secret.to_string(),
            bot_user_id,
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_secs(300))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Access the signing secret (needed by webhook handler for url_verification).
    pub fn signing_secret(&self) -> &str {
        &self.signing_secret
    }

    /// List Slack channels the bot is a member of.
    pub async fn list_channels(&self) -> Result<Vec<super::api::SlackChannelInfo>, ChannelError> {
        self.api.list_conversations().await
    }

    /// Parse a Slack Events API webhook into an InboundMessage.
    ///
    /// Handles: signature verification, url_verification challenge,
    /// trigger filtering, event deduplication, and user identity extraction.
    pub async fn parse_event(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        config: &ChannelConfig,
        db: &DbPool,
    ) -> Result<SlackWebhookResult, ChannelError> {
        // 1. Verify signature
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let sig = headers
            .get("x-slack-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        signature::verify_signature(&self.signing_secret, timestamp, body, sig)?;

        // 2. Parse body as JSON
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

        // 3. Handle url_verification challenge
        if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
            let challenge = payload
                .get("challenge")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return Ok(SlackWebhookResult::Challenge(challenge));
        }

        // 4. Extract event
        let event = match payload.get("event") {
            Some(e) => e,
            None => return Ok(SlackWebhookResult::Ignore),
        };

        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let event_id = payload
            .get("event_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // 5. Deduplication
        if !event_id.is_empty() {
            if self.dedup_cache.get(event_id).await.is_some() {
                debug!(event_id = %event_id, "Duplicate Slack event, ignoring");
                return Ok(SlackWebhookResult::Ignore);
            }
            self.dedup_cache.insert(event_id.to_string(), ()).await;
        }

        // 6. Parse typed provider config and apply trigger filter
        let slack_config = SlackProviderConfig::from_config(config);

        match (slack_config.trigger, event_type) {
            (ChannelTrigger::MentionOnly, "app_mention") => {} // process
            (ChannelTrigger::AllMessages, "message") => {}     // process
            (ChannelTrigger::AllMessages, "app_mention") => {} // also process mentions in all_messages mode
            _ => return Ok(SlackWebhookResult::Ignore),
        }

        // 7. Ignore bot's own messages
        let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
        let channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(ref bot_id) = self.bot_user_id {
            if user == bot_id {
                return Ok(SlackWebhookResult::Ignore);
            }
        }
        // Also check for bot_id field (different from user field)
        if event.get("bot_id").is_some() {
            return Ok(SlackWebhookResult::Ignore);
        }

        // 7.5. Enforce access policy
        match enforce_slack_access(&slack_config, channel, user) {
            AccessDecision::Allowed => {}
            AccessDecision::Blocked | AccessDecision::PairingSent => {
                debug!(channel, user, "Slack message blocked by access policy");
                return Ok(SlackWebhookResult::Ignore);
            }
        }

        // 8. Extract message details
        let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let ts = event.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        let thread_ts = event.get("thread_ts").and_then(|v| v.as_str());

        if text.is_empty() || ts.is_empty() {
            return Ok(SlackWebhookResult::Ignore);
        }

        // 9. Strip bot mention and convert Slack mrkdwn to readable text
        let clean_text = strip_bot_mention(text, self.bot_user_id.as_deref());
        let clean_text = super::format::format_slack_text(&clean_text);

        if clean_text.trim().is_empty() {
            return Ok(SlackWebhookResult::Ignore);
        }

        // 10. Resolve Slack user → system user
        let user_id = match self.resolve_user(user, db).await? {
            Some(uid) => uid,
            None if thread_ts.is_some() => {
                // Thread reply from unmapped user — fall back to config owner
                // (user identity only matters for task creation)
                info!(
                    slack_user = %user,
                    source_id = %channel,
                    fallback_user = %config.created_by,
                    "Slack user not mapped, using config owner for thread reply"
                );
                config.created_by.clone()
            }
            None => {
                // Top-level message from unmapped user — block and notify
                warn!(slack_user = %user, source_id = %channel, "Cannot process message: Slack user not mapped to system user");
                let email = self.api.get_user_email(user).await.ok().flatten();
                let error_msg = match email {
                    Some(ref email) => format!(
                        "Sorry, I couldn't find your account. Please make sure you're registered with the email address: {email}"
                    ),
                    None => "Sorry, I couldn't find your account. Please make sure you're registered in the system with the same email as your Slack profile.".to_string(),
                };
                let reply_ts = thread_ts.unwrap_or(ts);
                let _ = self
                    .api
                    .post_message(channel, &error_msg, Some(reply_ts))
                    .await;
                return Ok(SlackWebhookResult::Ignore);
            }
        };

        // Build thread properties
        let thread_properties = serde_json::json!({
            "channel": channel,
            "ts": ts,
            "thread_ts": thread_ts.unwrap_or(ts),
        });

        Ok(SlackWebhookResult::Message(InboundMessage {
            provider: ChannelType::Slack,
            source_id: channel.to_string(),
            thread_id: thread_ts.map(std::string::ToString::to_string),
            message_thread_id: thread_ts.unwrap_or(ts).to_string(),
            user_id,
            text: clean_text,
            file_ids: vec![], // TODO: handle file attachments
            thread_properties,
            provider_context: serde_json::json!({
                "team_id": payload.get("team_id").and_then(|v| v.as_str()).unwrap_or(""),
            }),
        }))
    }
}

#[async_trait::async_trait]
impl ChannelProvider for SlackProvider {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Slack
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        use tracing::info;

        // Look up Slack user email → find system user by email
        let email = self.api.get_user_email(identity).await?;
        let Some(email) = email else {
            warn!(slack_user = %identity, "Slack API returned no email for user");
            return Ok(None);
        };

        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT id, login FROM users WHERE email = $1 LIMIT 1",
        )
        .bind(&email)
        .fetch_optional(db)
        .await
        .map_err(|e| ChannelError::Internal(format!("User lookup failed: {e}")))?;

        if let Some((id, login)) = row {
            info!(slack_user = %identity, email = %email, user_id = %id, login = %login, "Resolved Slack user to system user");
            Ok(Some(id))
        } else {
            warn!(slack_user = %identity, email = %email, "No system user found for Slack user email");
            Ok(None)
        }
    }

    async fn send_reply(
        &self,
        thread: &ChannelThread,
        _config: &ChannelConfig,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        let channel = thread
            .properties
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Internal("No channel in thread properties".into()))?;
        let thread_ts = thread
            .properties
            .get("thread_ts")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Internal("No thread_ts in thread properties".into()))?;

        let mut text = message.text.clone();
        if let Some(ref url) = message.task_url {
            use std::fmt::Write;
            let _ = write!(text, "\n<{url}|View task>");
        }

        let ts = self
            .api
            .post_message(channel, &text, Some(thread_ts))
            .await?;
        Ok(ts)
    }

    async fn send_message(
        &self,
        config: &ChannelConfig,
        message: &OutboundReply,
    ) -> Result<NewThreadInfo, ChannelError> {
        let channel = &config.source_id;

        let mut text = message.text.clone();
        if let Some(ref url) = message.task_url {
            use std::fmt::Write;
            let _ = write!(text, "\n<{url}|View task>");
        }

        let ts = self
            .api
            .post_message(channel, &text, None)
            .await?
            .ok_or_else(|| {
                ChannelError::Internal("Slack did not return message ts".into())
            })?;

        Ok(NewThreadInfo {
            external_thread_id: ts.clone(),
            properties: serde_json::json!({
                "channel": channel,
                "ts": ts,
                "thread_ts": ts,
            }),
        })
    }

    async fn set_status(
        &self,
        thread: &ChannelThread,
        config: &ChannelConfig,
        status: MessageStatus,
    ) -> Result<(), ChannelError> {
        let channel = thread
            .properties
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ts = thread
            .properties
            .get("ts")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if channel.is_empty() || ts.is_empty() {
            return Ok(());
        }

        let slack_config = SlackProviderConfig::from_config(config);
        let reactions = slack_config.reactions.as_ref();

        let emoji = match (status, reactions) {
            (MessageStatus::Processing, Some(r)) => {
                r.get("processing").and_then(|v| v.as_str())
            }
            (MessageStatus::Completed, Some(r)) => {
                r.get("completed").and_then(|v| v.as_str())
            }
            (MessageStatus::Error, Some(r)) => r.get("error").and_then(|v| v.as_str()),
            (MessageStatus::PendingResponse, Some(r)) => {
                r.get("pending").and_then(|v| v.as_str())
            }
            // Default reactions
            (MessageStatus::Processing, None) => Some("eyes"),
            (MessageStatus::Completed, None) => Some("white_check_mark"),
            (MessageStatus::Error, None) => Some("x"),
            (MessageStatus::PendingResponse, None) => Some("question"),
        };

        if let Some(emoji) = emoji {
            // For completed/error/pending, first remove processing reaction
            if !matches!(status, MessageStatus::Processing) {
                let processing_emoji = reactions
                    .and_then(|r| r.get("processing"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("eyes");
                let _ = self.api.remove_reaction(channel, ts, processing_emoji).await;
            }

            self.api.add_reaction(channel, ts, emoji).await?;
        }

        Ok(())
    }

    async fn fetch_context(
        &self,
        thread: &ChannelThread,
        config: &ChannelConfig,
        limit: usize,
    ) -> Result<Option<String>, ChannelError> {
        let channel = thread
            .properties
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let thread_ts = thread
            .properties
            .get("thread_ts")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if channel.is_empty() || thread_ts.is_empty() {
            return Ok(None);
        }

        let slack_config = SlackProviderConfig::from_config(config);
        let config_limit = slack_config
            .thread_context_limit
            .unwrap_or(limit as u64) as usize;

        let messages = self
            .api
            .get_thread_replies(
                channel,
                thread_ts,
                config_limit,
                self.bot_user_id.as_deref(),
            )
            .await?;

        if messages.is_empty() {
            return Ok(None);
        }

        Ok(Some(messages.join("\n\n---\n\n")))
    }

}

/// Strip bot mention from message text (e.g., "<@U123ABC> fix the bug" → "fix the bug").
fn strip_bot_mention(text: &str, bot_user_id: Option<&str>) -> String {
    if let Some(bot_id) = bot_user_id {
        let mention = format!("<@{bot_id}>");
        text.replace(&mention, "").trim().to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_mention() {
        let text = "<@U123ABC> fix the bug";
        assert_eq!(strip_bot_mention(text, Some("U123ABC")), "fix the bug");
    }

    #[test]
    fn strip_mention_no_bot_id() {
        let text = "<@U123ABC> fix the bug";
        assert_eq!(strip_bot_mention(text, None), "<@U123ABC> fix the bug");
    }

    #[test]
    fn strip_mention_no_mention_in_text() {
        let text = "fix the bug";
        assert_eq!(strip_bot_mention(text, Some("U123ABC")), "fix the bug");
    }
}
