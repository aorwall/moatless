//! Slack channel provider — ChannelProvider trait impl + webhook parsing.

use axum::http::HeaderMap;
use moka::future::Cache;
use std::fmt::Write as _;
use std::time::Duration;
use tokio::sync::OnceCell;
use tracing::{debug, info, warn};

use crate::channel::error::ChannelError;
use crate::channel::models::{AccessDecision, ChannelTrigger};
use crate::channel::models::{
    BindingContext, ProviderConnection, TaskBinding, TaskTriggerRoute, TriggerContext,
};
use crate::channel::models::{InboundMessage, NewThreadInfo, OutboundReply};
use crate::channel::provider::{
    BindingDestination, ChannelProvider, ChannelType, DestinationPostResult,
};
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
- Images will render inline in the Slack thread.

### Replying to the same Slack thread next turn
After your first `moat channels send` (or `moat reply`) succeeds, the CLI prints a \
`--ts <binding_id>` hint. Use it on follow-ups so your reply lands in the same \
thread instead of fanning out:
- `moat channels send --ts <binding_id> --text \"...\"`
- `moat channels bindings --task $TASK_ID` lists active targets if you lost the id.",
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
    /// Workspace (team) id memoized after the first `auth.test` call so
    /// `post_to_destination` doesn't ping Slack on every send.
    workspace_id: OnceCell<String>,
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
            workspace_id: OnceCell::new(),
        }
    }

    /// Return the Slack workspace (team) ID, calling `auth.test` once and
    /// memoizing the result for the lifetime of the provider.
    async fn ensure_workspace_id(&self) -> Result<String, ChannelError> {
        self.workspace_id
            .get_or_try_init(|| async {
                let info = self.api.auth_test().await?;
                info.team_id.ok_or_else(|| {
                    ChannelError::ProviderApi("Slack auth.test returned no team_id".into())
                })
            })
            .await
            .cloned()
    }

    /// Access the signing secret (needed by webhook handler for url_verification).
    pub fn signing_secret(&self) -> &str {
        &self.signing_secret
    }

    /// Resolve the identity of the current Slack bot token via `auth.test`.
    /// The returned `team_id` is the Slack workspace ID (e.g. "T0000000002"),
    /// needed to build `task_bindings.subject_key` for DM replies.
    pub async fn auth_test(&self) -> Result<super::api::AuthTestInfo, ChannelError> {
        self.api.auth_test().await
    }

    /// List Slack channels the bot is a member of.
    pub async fn list_channels(&self) -> Result<Vec<super::api::SlackChannelInfo>, ChannelError> {
        self.api.list_conversations().await
    }

    /// Open a DM channel with a Slack user and return the channel ID.
    pub async fn open_dm(&self, slack_user_id: &str) -> Result<String, ChannelError> {
        self.api.open_dm(slack_user_id).await
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

    /// Look up a local system user by their Slack user ID via `user_provider_ids`.
    async fn resolve_user_by_slack_id(
        &self,
        slack_user_id: &str,
        db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"SELECT u.id FROM users u
               JOIN user_provider_ids p ON u.id = p.user_id
               WHERE p.provider = 'slack' AND p.provider_user_id = $1
               LIMIT 1"#,
        )
        .bind(slack_user_id)
        .fetch_optional(db)
        .await
        .map_err(|e| ChannelError::Internal(format!("Slack provider ID lookup failed: {e}")))?;

        if let Some((id,)) = row {
            tracing::debug!(
                slack_user = %slack_user_id,
                user_id = %id,
                "Resolved Slack user via provider_id mapping"
            );
            Ok(Some(id))
        } else {
            Ok(None)
        }
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
        config: &TriggerContext,
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
        let event = if let Some(e) = payload.get("event") {
            e
        } else {
            info!(reason = "no_event_field", "Slack event ignored");
            return Ok(SlackWebhookResult::Ignore);
        };

        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let event_subtype = event.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
        let event_id = payload
            .get("event_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // 5. Deduplication
        if !event_id.is_empty() {
            if self.dedup_cache.get(event_id).await.is_some() {
                info!(
                    reason = "duplicate_event_id",
                    event_id = %event_id,
                    event_type = %event_type,
                    "Slack event ignored"
                );
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
                info!(
                    reason = "user_is_own_bot",
                    event_type = %event_type,
                    user = %user,
                    source_id = %channel,
                    "Slack event ignored"
                );
                return Ok(SlackWebhookResult::Ignore);
            }
        }
        // Also check for bot_id field (different from user field)
        if let Some(bot_id_value) = event.get("bot_id") {
            info!(
                reason = "bot_id_field_present",
                event_type = %event_type,
                event_subtype = %event_subtype,
                user = %user,
                bot_id = %bot_id_value,
                source_id = %channel,
                "Slack event ignored"
            );
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
            _ => {
                info!(
                    reason = "trigger_filter",
                    event_type = %event_type,
                    event_subtype = %event_subtype,
                    trigger = ?slack_config.trigger,
                    is_tracked_thread,
                    is_dm,
                    user = %user,
                    source_id = %channel,
                    "Slack event ignored"
                );
                return Ok(SlackWebhookResult::Ignore);
            }
        }

        // 9. Enforce access policy
        match enforce_slack_access(&slack_config, channel, user) {
            AccessDecision::Allowed => {}
            AccessDecision::Blocked | AccessDecision::PairingSent => {
                info!(
                    reason = "access_policy",
                    event_type = %event_type,
                    user = %user,
                    source_id = %channel,
                    "Slack event ignored"
                );
                return Ok(SlackWebhookResult::Ignore);
            }
        }

        // 8. Extract message details
        let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let ts = event.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        let thread_ts = event.get("thread_ts").and_then(|v| v.as_str());

        if text.is_empty() || ts.is_empty() {
            info!(
                reason = "empty_text_or_ts",
                event_type = %event_type,
                text_empty = text.is_empty(),
                ts_empty = ts.is_empty(),
                user = %user,
                source_id = %channel,
                "Slack event ignored"
            );
            return Ok(SlackWebhookResult::Ignore);
        }

        // 9. Detect bot mention BEFORE stripping (needed for selective reply logic)
        let is_bot_mentioned = event_type == "app_mention"
            || self
                .bot_user_id
                .as_ref()
                .is_some_and(|bid| text.contains(&format!("<@{bid}>")));

        // Convert Slack mrkdwn to readable text. We deliberately do NOT strip
        // the bot's own `<@bot_id>` mention — the agent's system prompt
        // (`build_slack_reply_instructions`) already explains that token.
        // Stripping previously caused bare `@bot` replies to be silently
        // dropped because the post-strip text was empty.
        let clean_text = super::format::format_slack_text(text);

        if clean_text.trim().is_empty() {
            info!(
                reason = "empty_text_after_format",
                event_type = %event_type,
                raw_text_len = text.len(),
                is_bot_mentioned,
                user = %user,
                source_id = %channel,
                "Slack event ignored"
            );
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

        // Try provider ID lookup first (fast, avoids extra Slack API call), then email.
        let mapped_user = self.resolve_user_by_slack_id(user, db).await?.or({
            match slack_user_info.as_ref().and_then(|i| i.email.clone()) {
                Some(ref email) => self.resolve_user_by_email(user, email, db).await?,
                None => None,
            }
        });

        // Opportunistically cache the Slack user ID for future provider ID lookups.
        if let Some(ref uid) = mapped_user {
            match sqlx::query(
                r#"INSERT INTO user_provider_ids (user_id, provider, provider_user_id)
                   VALUES ($1, 'slack', $2)
                   ON CONFLICT (user_id, provider) DO NOTHING"#,
            )
            .bind(uid)
            .bind(user)
            .execute(db)
            .await
            {
                Ok(result) if result.rows_affected() > 0 => {
                    tracing::info!(
                        user_id = %uid,
                        slack_user = %user,
                        "Cached Slack user ID in user_provider_ids"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(
                        error = %e,
                        user_id = %uid,
                        slack_user = %user,
                        "Failed to cache Slack provider ID in user_provider_ids"
                    );
                }
            }
        }

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
        let team_id = payload
            .get("team_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("default");
        let thread_properties = serde_json::json!({
            "channel": channel,
            "ts": ts,
            "thread_ts": thread_ts.unwrap_or(ts),
            "team_id": team_id,
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
                "team_id": team_id,
                "user_id": user,
                "is_bot_mentioned": is_bot_mentioned,
            }),
            channel_external_id: Some(super::external_id::message_ts(ts)),
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

    fn binding_destination_from_existing_thread(
        &self,
        source_id: &str,
        external_thread_id: &str,
        thread_properties: &serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, ChannelError> {
        let channel = thread_properties
            .get("channel")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(source_id);
        let thread_ts = thread_properties
            .get("thread_ts")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(external_thread_id);
        let team = thread_properties
            .get("team_id")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("default");
        let ts = thread_properties
            .get("ts")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(thread_ts);
        let mut destination_config = serde_json::json!({
            "channel": channel,
            "thread_ts": thread_ts,
            "ts": ts,
        });
        if let Some(filter_user_id) = filter_user_id {
            destination_config["filter_user_id"] = serde_json::json!(filter_user_id);
        }

        Ok(BindingDestination::new(
            "slack_thread",
            format!("{team}:{channel}:{thread_ts}"),
            destination_config,
        ))
    }

    fn binding_destination_from_new_thread(
        &self,
        rule: &TaskTriggerRoute,
        new_thread_info: &NewThreadInfo,
    ) -> Result<BindingDestination, ChannelError> {
        let destination = self.binding_destination_from_existing_thread(
            &rule.source_matcher,
            &new_thread_info.external_thread_id,
            &new_thread_info.properties,
            None,
        )?;
        Ok(destination.with_last_external_event_id(new_thread_info.external_thread_id.clone()))
    }

    fn sender_label_suffix(&self, msg: &InboundMessage) -> Option<String> {
        let provider_user_id = msg
            .provider_context
            .get("user_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())?;
        Some(format!(", Slack: {provider_user_id}"))
    }

    fn inbound_message_prefix(&self, msg: &InboundMessage) -> Option<String> {
        msg.provider_context
            .get("is_bot_mentioned")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            .then(|| "[You were @mentioned - a reply via `moat reply` is required]".to_string())
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
        thread: &BindingContext,
        _config: &TriggerContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        let channel = thread
            .destination_config
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Internal("No channel in thread properties".into()))?;
        let thread_ts = thread
            .destination_config
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

        Ok(ts.map(|t| super::external_id::message_ts(&t)))
    }

    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        _connection: &ProviderConnection,
        rule: Option<&TaskTriggerRoute>,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        let thread = crate::channel::binding_runtime::binding_context(binding);
        let config = rule.map_or_else(
            || TriggerContext {
                id: binding.trigger_id.clone().unwrap_or_default(),
                provider: "slack".to_string(),
                source_id: binding
                    .destination_config
                    .get("channel")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                source_name: None,
                repository_id: String::new(),
                routing_mode: crate::channel::models::RoutingMode::BySubject,
                prompt_template: None,
                active: true,
                allow_direct_send: false,
                provider_config: None,
                ongoing_task_id: None,
                scope: crate::scope::Scope::Global,
                team_id: None,
                task_name: None,
                tag_ids: vec![],
                run_as_user_id: None,
                agent_type: "claude-code".to_string(),
                created_by: "system".to_string(),
                created_at: binding.created_at,
                updated_at: binding.last_event_at,
            },
            |r| crate::channel::binding_runtime::trigger_context(r, "slack"),
        );
        self.send_reply(&thread, &config, message).await
    }

    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<DestinationPostResult, ChannelError> {
        let workspace_id = self.ensure_workspace_id().await?;

        let ts = self
            .api
            .post_message(destination, &message.text, None)
            .await?
            .ok_or_else(|| ChannelError::Internal("Slack did not return message ts".into()))?;

        for file in &message.files {
            if let Err(e) = self
                .api
                .upload_file(destination, Some(&ts), &file.filename, file.data.clone())
                .await
            {
                warn!(
                    filename = %file.filename,
                    error = %e,
                    "Failed to upload file attachment to Slack"
                );
            }
        }

        Ok(DestinationPostResult {
            message_external_id: ts.clone(),
            subject_kind: "slack_thread",
            subject_key: format!("{workspace_id}:{destination}:{ts}"),
            destination_config: serde_json::json!({
                "channel": destination,
                "ts": ts,
                "thread_ts": ts,
            }),
        })
    }

    async fn send_message(
        &self,
        config: &TriggerContext,
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
        thread: &BindingContext,
        config: &TriggerContext,
        limit: usize,
        exclude_external_ids: &[String],
    ) -> Result<Option<String>, ChannelError> {
        let channel = thread
            .destination_config
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let thread_ts = thread
            .destination_config
            .get("thread_ts")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if channel.is_empty() || thread_ts.is_empty() {
            return Ok(None);
        }

        let slack_config = SlackProviderConfig::from_config(config);
        let config_limit = slack_config.thread_context_limit.unwrap_or(limit as u64) as usize;

        // Use last_external_event_id as cursor — only fetch messages after it.
        // Strip the `slack:ts:` namespace prefix written by `send_reply` so the
        // Slack API still receives a bare ts.
        let oldest_raw = thread
            .destination_config
            .get("last_external_event_id")
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
                    let prefixed = super::external_id::message_ts(ts);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::models::{RoutingMode, TaskTriggerRoute};
    use crate::channel::provider::ChannelProvider;
    use crate::scope::Scope;

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

    #[test]
    fn binding_destination_from_existing_thread_uses_workspace_channel_and_thread() {
        let provider = SlackProvider::new("token", "secret", None);
        let destination = provider
            .binding_destination_from_existing_thread(
                "C123",
                "1700000000.000100",
                &serde_json::json!({
                    "channel": "C123",
                    "ts": "1700000000.000100",
                    "thread_ts": "1700000000.000100",
                    "team_id": "T999",
                }),
                Some("user-1"),
            )
            .unwrap();

        assert_eq!(destination.subject_kind, "slack_thread");
        assert_eq!(destination.subject_key, "T999:C123:1700000000.000100");
        assert_eq!(destination.destination_config["channel"], "C123");
        assert_eq!(
            destination.destination_config["thread_ts"],
            "1700000000.000100"
        );
        assert_eq!(destination.destination_config["filter_user_id"], "user-1");
        assert_eq!(destination.last_external_event_id, None);
    }

    #[test]
    fn binding_destination_from_new_thread_preserves_direct_send_shape() {
        let provider = SlackProvider::new("token", "secret", None);
        let rule = test_rule("slack", "C123");
        let destination = provider
            .binding_destination_from_new_thread(
                &rule,
                &NewThreadInfo {
                    external_thread_id: "1700000000.000100".to_string(),
                    properties: serde_json::json!({
                        "channel": "C123",
                        "ts": "1700000000.000100",
                        "thread_ts": "1700000000.000100",
                    }),
                },
            )
            .unwrap();

        assert_eq!(destination.subject_kind, "slack_thread");
        assert_eq!(destination.subject_key, "default:C123:1700000000.000100");
        assert_eq!(
            destination.destination_config,
            serde_json::json!({
                "channel": "C123",
                "ts": "1700000000.000100",
                "thread_ts": "1700000000.000100",
            })
        );
        assert_eq!(
            destination.last_external_event_id.as_deref(),
            Some("1700000000.000100")
        );
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
            repository_id: "repo-1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            allow_direct_send: true,
            provider_config: None,
            ongoing_task_id: None,
            close_task_on_close: true,
            notify_on_success: false,
            active: true,
            created_by: "user-1".into(),
            scope: Scope::Global,
            team_id: None,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}
