//! Slack channel provider — ChannelProvider trait impl + webhook parsing.

use axum::http::HeaderMap;
use moka::future::Cache;
use std::fmt::Write as _;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::channel::error::ChannelError;
use crate::channel::external_id;
use crate::channel::models::{AccessDecision, ChannelTrigger};
use crate::channel::models::{
    ChannelConfig, ChannelThread, IngressRule, ProviderConnection, TaskSubscription,
};
use crate::channel::models::{InboundMessage, NewThreadInfo, OutboundReply};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::db::DbPool;

use super::access::enforce_slack_access;
use super::api::SlackApi;
use super::config::SlackProviderConfig;
use super::signature;

/// Build Slack-specific reply instructions, with the bot's own Slack identity
/// stated up-front so the agent always knows who it is in the thread.
fn build_slack_reply_instructions(bot_user_id: Option<&str>) -> String {
    let mut s = String::from("## Slack-specific instructions\n\n### Your identity\n");

    match bot_user_id {
        Some(bot_id) => {
            let _ = writeln!(
                s,
                "You are posting to Slack as user `<@{bot_id}>` (Slack user ID `{bot_id}`). \
Any message from `<@{bot_id}>` in the thread is something you already said — do not treat it as a new request. \
When someone writes `<@{bot_id}>` they are addressing you directly."
            );
        }
        None => {
            s.push_str(
                "Your Slack user ID is not configured. \
Rely on sender labels (`Name (login, Slack: U...): ...`) to tell yourself apart from other participants, \
and when in doubt stay silent rather than replying to your own prior messages.\n",
            );
        }
    }

    s.push_str(
        "\n\
### Mentioning users
The message includes the sender's identity (e.g. \"Name (login, Slack: U12345): ...\"). \
To mention them in your reply, use their Slack user ID with the format `<@U12345>`.

### Attaching files
To include files (screenshots, logs, generated artifacts) in your reply, use the `--file` flag:
- `moat reply --text \"Here are the results\" --file screenshot.png`
- `moat reply --text \"Logs attached\" --file output.log --file error.log`
- Multiple `--file` flags can be used to attach several files at once.
- Images will render inline in the Slack thread.",
    );

    if let Some(bot_id) = bot_user_id {
        let _ = write!(
            s,
            "\n\n\
### When to respond in thread conversations\n\
You will receive ALL messages in the thread, not just ones directed at you. \
A message containing `<@{bot_id}>` or your handle references you, but that alone does not mean it's addressed to you — \
people often mention the bot in passing (\"look at what @you said\", \"ask @you later\") without actually asking you anything.\n\
- **Respond** when the message is addressed *to* you — a question, request, or instruction aimed at you\n\
- **Respond** when someone asks a question or requests help only you can provide, even without a mention\n\
- **Stay silent** when you are merely referenced or named in passing — the conversation is between other people\n\
- **Stay silent** when people are talking to each other, for acknowledgments, reactions, or off-topic chat\n\
- When in doubt, stay silent rather than interjecting\n\
- To stay silent, simply do nothing — do NOT send a `moat reply`"
        );
    }

    s
}

/// Result of parsing a Slack webhook event.
pub enum SlackWebhookResult {
    /// URL verification challenge — return the challenge string immediately.
    Challenge(String),
    /// Parsed message ready for routing.
    Message(Box<InboundMessage>),
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

    /// Post a message to a Slack channel, returning the message `ts`.
    ///
    /// Thin wrapper around [`SlackApi::post_message`] for use by the slash
    /// command handler (which needs to post a visible thread anchor).
    pub async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<Option<String>, ChannelError> {
        self.api.post_message(channel, text, thread_ts).await
    }

    /// Resolve a Slack channel ID to its human-readable name (best-effort).
    pub async fn get_channel_name(&self, channel_id: &str) -> Result<Option<String>, ChannelError> {
        Ok(self
            .api
            .get_channel_info(channel_id)
            .await?
            .map(|ch| ch.name))
    }

    /// Look up a local system user by email. Shared by the `ChannelProvider`
    /// trait impl and by `parse_event` (which also inspects bot flags).
    async fn resolve_user_by_email(
        &self,
        identity: &str,
        email: &str,
        db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT id, login FROM users WHERE email = $1 LIMIT 1")
                .bind(email)
                .fetch_optional(db)
                .await
                .map_err(|e| ChannelError::Internal(format!("User lookup failed: {e}")))?;

        if let Some((id, login)) = row {
            tracing::info!(slack_user = %identity, email = %email, user_id = %id, login = %login, "Resolved Slack user to system user");
            Ok(Some(id))
        } else {
            warn!(slack_user = %identity, email = %email, "No system user found for Slack user email");
            Ok(None)
        }
    }

    /// Parse a Slack Events API webhook into an InboundMessage.
    ///
    /// Handles: signature verification, url_verification challenge,
    /// trigger filtering, event deduplication, and user identity extraction.
    ///
    /// When `is_tracked_thread` is true, the trigger filter is bypassed for
    /// `message` events — this allows thread replies to outbound-created threads
    /// to be processed regardless of the config's trigger mode.
    pub async fn parse_event(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        config: &ChannelConfig,
        db: &DbPool,
        is_tracked_thread: bool,
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

        // 6. Extract user + channel early (needed for bot filter, DM detection, and access policy)
        let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
        let channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");

        // 7. Ignore bot's own messages
        if let Some(ref bot_id) = self.bot_user_id {
            if user == bot_id {
                return Ok(SlackWebhookResult::Ignore);
            }
        }
        // Also check for bot_id field (different from user field)
        if event.get("bot_id").is_some() {
            return Ok(SlackWebhookResult::Ignore);
        }

        // 8. Parse typed provider config and apply trigger filter
        let slack_config = SlackProviderConfig::from_config(config);

        // DM channels (starting with "D") are always directed at the bot —
        // bypass trigger filter for them.
        let is_dm = channel.starts_with('D');

        match (slack_config.trigger, event_type) {
            (ChannelTrigger::MentionOnly, "app_mention") => {} // process
            (ChannelTrigger::AllMessages, "message") => {}     // process
            (ChannelTrigger::AllMessages, "app_mention") => {} // also process mentions in all_messages mode
            // Always process thread replies to threads we're tracking (e.g. outbound-created)
            (_, "message") if is_tracked_thread => {
                debug!(event_id = %event_id, "Processing tracked thread reply (trigger bypass)");
            }
            // DMs are inherently directed at the bot — always process
            (_, "message") if is_dm => {
                debug!(event_id = %event_id, channel = %channel, "Processing DM (trigger bypass)");
            }
            _ => return Ok(SlackWebhookResult::Ignore),
        }

        // 9. Enforce access policy
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

        // 9. Detect bot mention BEFORE stripping (needed for selective reply logic)
        let is_bot_mentioned = event_type == "app_mention"
            || self
                .bot_user_id
                .as_ref()
                .is_some_and(|bid| text.contains(&format!("<@{bid}>")));

        // Strip bot mention and convert Slack mrkdwn to readable text
        let clean_text = strip_bot_mention(text, self.bot_user_id.as_deref());
        let clean_text = super::format::format_slack_text(&clean_text);

        if clean_text.trim().is_empty() {
            return Ok(SlackWebhookResult::Ignore);
        }

        // 10. Fetch user info once (bot flags + email) and resolve Slack user → system user.
        let slack_user_info = self.api.get_user_info(user).await.ok().flatten();

        // Drop bot/app-account senders: their messages should not drive task flow,
        // and — critically — they share emails across deployments so the email-based
        // user resolution is unsafe for them. This catches external deployment bots
        // that slip past the earlier `bot_id` field filter.
        if let Some(ref info) = slack_user_info {
            if info.is_bot || info.is_app_user {
                let is_own_bot = self.bot_user_id.as_deref() == Some(user);
                info!(
                    slack_user = %user,
                    source_id = %channel,
                    is_own_bot,
                    handle = %info.handle.as_deref().unwrap_or(""),
                    "Ignoring Slack message from bot/app account"
                );
                return Ok(SlackWebhookResult::Ignore);
            }
        }

        let mapped_user = match slack_user_info.as_ref().and_then(|i| i.email.clone()) {
            Some(email) => self.resolve_user_by_email(user, &email, db).await?,
            None => None,
        };

        let user_id = match mapped_user {
            Some(uid) => uid,
            None if thread_ts.is_some() => {
                // Thread reply from unmapped human — fall back to config owner
                // (user identity only matters for task creation)
                info!(
                    slack_user = %user,
                    source_id = %channel,
                    fallback_user = %config.effective_user_id(),
                    "Slack user not mapped, using config owner for thread reply"
                );
                config.effective_user_id().to_string()
            }
            None => {
                // Top-level message from unmapped human — block and notify
                warn!(slack_user = %user, source_id = %channel, "Cannot process message: Slack user not mapped to system user");
                let email = slack_user_info.as_ref().and_then(|i| i.email.clone());
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

        Ok(SlackWebhookResult::Message(Box::new(InboundMessage {
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
                "user_id": user,
                "is_bot_mentioned": is_bot_mentioned,
            }),
            channel_external_id: Some(external_id::slack_ts(ts)),
        })))
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

    fn supports_files(&self) -> bool {
        true
    }

    fn reply_instructions(&self) -> Option<String> {
        Some(build_slack_reply_instructions(self.bot_user_id.as_deref()))
    }

    fn thread_reply_hint(&self, msg: &InboundMessage) -> Option<String> {
        let is_bot_mentioned = msg
            .provider_context
            .get("is_bot_mentioned")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if is_bot_mentioned {
            None
        } else {
            Some("[New message in Slack thread]".to_string())
        }
    }

    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        let Some(info) = self.api.get_user_info(identity).await? else {
            warn!(slack_user = %identity, "Slack users.info returned no user");
            return Ok(None);
        };

        // Bot / app accounts share emails across deployments — never resolve them by email.
        if info.is_bot || info.is_app_user {
            tracing::info!(
                slack_user = %identity,
                is_bot = info.is_bot,
                is_app_user = info.is_app_user,
                handle = %info.handle.as_deref().unwrap_or(""),
                "Slack user is a bot/app account — skipping email-based user mapping"
            );
            return Ok(None);
        }

        let Some(email) = info.email else {
            warn!(slack_user = %identity, "Slack API returned no email for user");
            return Ok(None);
        };

        self.resolve_user_by_email(identity, &email, db).await
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

        // Upload file attachments (best-effort: log failures, don't fail the reply)
        for file in &message.files {
            if let Err(e) = self
                .api
                .upload_file(channel, Some(thread_ts), &file.filename, file.data.clone())
                .await
            {
                warn!(
                    filename = %file.filename,
                    error = %e,
                    "Failed to upload file attachment to Slack"
                );
            }
        }

        Ok(ts.map(|t| external_id::slack_ts(&t)))
    }

    async fn send_reply_for_subscription(
        &self,
        subscription: &TaskSubscription,
        _connection: &ProviderConnection,
        rule: Option<&IngressRule>,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        let thread = crate::channel::subscription::synthetic_thread(subscription);
        let config = rule.map_or_else(
            || ChannelConfig {
                id: subscription.ingress_rule_id.clone().unwrap_or_default(),
                provider: "slack".to_string(),
                source_id: subscription
                    .reply_target
                    .get("channel")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                source_name: None,
                repository_id: String::new(),
                routing_mode: crate::channel::models::RoutingMode::ThreadBased,
                prompt_template: None,
                active: true,
                allow_direct_send: false,
                provider_config: None,
                ongoing_task_id: None,
                response_config_id: None,
                scope: crate::scope::Scope::Global,
                team_id: None,
                task_name: None,
                tag_ids: vec![],
                run_as_user_id: None,
                agent_type: "claude-code".to_string(),
                created_by: "system".to_string(),
                created_at: subscription.created_at,
                updated_at: subscription.last_event_at,
            },
            |r| crate::channel::subscription::synthetic_config(r, "slack"),
        );
        self.send_reply(&thread, &config, message).await
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
            .ok_or_else(|| ChannelError::Internal("Slack did not return message ts".into()))?;

        // Upload file attachments (best-effort: log failures, don't fail the message)
        for file in &message.files {
            if let Err(e) = self
                .api
                .upload_file(channel, Some(&ts), &file.filename, file.data.clone())
                .await
            {
                warn!(
                    filename = %file.filename,
                    error = %e,
                    "Failed to upload file attachment to Slack"
                );
            }
        }

        Ok(NewThreadInfo {
            external_thread_id: ts.clone(),
            properties: serde_json::json!({
                "channel": channel,
                "ts": ts,
                "thread_ts": ts,
            }),
        })
    }

    async fn fetch_context(
        &self,
        thread: &ChannelThread,
        config: &ChannelConfig,
        limit: usize,
        exclude_external_ids: &[String],
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
        let config_limit = slack_config.thread_context_limit.unwrap_or(limit as u64) as usize;

        // Use last_channel_external_id as cursor — only fetch messages after it.
        // Strip the `slack:ts:` namespace prefix written by `send_reply` so the
        // Slack API still receives a bare ts.
        let oldest_raw = thread
            .properties
            .get("last_channel_external_id")
            .and_then(|v| v.as_str());
        let oldest = oldest_raw.map(|s| s.strip_prefix("slack:ts:").unwrap_or(s));

        let replies = self
            .api
            .get_thread_replies(
                channel,
                thread_ts,
                config_limit,
                self.bot_user_id.as_deref(),
                oldest,
            )
            .await?;

        if replies.is_empty() {
            return Ok(None);
        }

        // Slack always returns the parent message regardless of `oldest`,
        // so filter client-side to only include messages strictly after the cursor.
        // Also exclude messages already delivered to the task (e.g. the current
        // message being processed) to avoid duplicates in the context. Match
        // both new prefixed and legacy bare-ts forms.
        let messages: Vec<&str> = replies
            .iter()
            .filter(|r| match (&r.ts, oldest) {
                (Some(ts), Some(cursor)) => ts.as_str() > cursor,
                _ => true,
            })
            .filter(|r| match &r.ts {
                Some(ts) => {
                    let prefixed = external_id::slack_ts(ts);
                    !exclude_external_ids.contains(&prefixed) && !exclude_external_ids.contains(ts)
                }
                None => true,
            })
            .map(|r| r.text.as_str())
            .collect();

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

    #[test]
    fn reply_instructions_include_identity_with_bot_id() {
        let s = build_slack_reply_instructions(Some("U0000000002"));
        // Identity section is present and appears before the mentioning-users block.
        let identity_idx = s
            .find("### Your identity")
            .expect("Your identity section present");
        let mentioning_idx = s
            .find("### Mentioning users")
            .expect("Mentioning users section present");
        assert!(
            identity_idx < mentioning_idx,
            "identity must come before mentioning-users"
        );
        // The identity block names the bot's Slack ID explicitly.
        assert!(s.contains("<@U0000000002>"));
        assert!(s.contains("Slack user ID `U0000000002`"));
        // Response guidance still references the ID.
        assert!(s.contains("### When to respond"));
    }

    #[test]
    fn reply_instructions_fallback_when_bot_id_missing() {
        let s = build_slack_reply_instructions(None);
        assert!(s.contains("### Your identity"));
        assert!(s.contains("not configured"));
        // Without a known ID the response-guidance block is skipped.
        assert!(!s.contains("### When to respond"));
    }
}
