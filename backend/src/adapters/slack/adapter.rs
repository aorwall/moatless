//! Slack adapter — Adapter trait impl + webhook parsing.

use axum::http::HeaderMap;
use moka::future::Cache;
use std::fmt::Write as _;
use std::time::Duration;
use tokio::sync::OnceCell;
use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{AccessDecision, MessageTrigger};
use crate::adapters::models::{
    AdapterConnection, BindingContext, LoopContext, SubscriptionMatch, TaskBinding,
};
use crate::adapters::models::{InboundMessage, NewThreadInfo, OutboundReply};
use crate::adapters::{Adapter, AdapterKind, BindingDestination, DestinationPostResult};
use crate::db::DbPool;

use super::access::enforce_slack_access;
use super::api::SlackApi;
use super::config::SlackAdapterConfig;
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
Inbound messages carry the sender's moatless login in a `<sent_by>...</sent_by>` \
header. To @-mention a Slack user in your reply, use their Slack user id with the \
format `<@U12345>`.

### Telling threads apart
Each inbound message also carries a `<thread>...</thread>` header identifying \
the Slack thread it belongs to. Messages sharing the same `<thread>` value are \
the same conversation; different values are separate threads. Use it to keep \
concurrent conversations distinct and to recognise a continuation of an earlier thread.

### Replying to a specific message
Every inbound message has a `<message_id>...</message_id>` header. Pass that id \
to `moat messages reply` so the reply lands in the right Slack thread \
(it threads to the right place automatically):
- `moat messages reply --message <id> --text \"Here are the results\"`

### Attaching files
Add `--file` flags to attach screenshots, logs, or generated artifacts to the reply:
- `moat messages reply --message <id> --text \"Here are the results\" --file screenshot.png`
- `moat messages reply --message <id> --text \"Logs attached\" --file output.log --file error.log`
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

/// Slack `message` events carry a `subtype` for system notifications — channel
/// join/leave, topic/purpose/name changes, archives, pins, edits, deletes, … —
/// rather than genuine user input. These must never be treated as task triggers:
/// in `all_messages` mode an unfiltered `channel_join` ("X has joined the
/// channel") otherwise spawns a task for every new member.
///
/// Deliberately absent:
/// - `bot_message` — foreign-bot posts are real content, gated separately by
///   `allow_bot_messages` (and the `bot_id` field), so they flow through here.
/// - `file_share`, `thread_broadcast`, `me_message` — these subtypes DO carry
///   user input and must continue to be processed.
fn is_ignored_message_subtype(subtype: &str) -> bool {
    matches!(
        subtype,
        // Channel (public) membership / metadata lifecycle
        "channel_join"
            | "channel_leave"
            | "channel_topic"
            | "channel_purpose"
            | "channel_name"
            | "channel_archive"
            | "channel_unarchive"
            // Private channel (group) equivalents
            | "group_join"
            | "group_leave"
            | "group_topic"
            | "group_purpose"
            | "group_name"
            | "group_archive"
            | "group_unarchive"
            // Edits, deletes, pins — not new user input
            | "message_changed"
            | "message_deleted"
            | "message_replied"
            | "pinned_item"
            | "unpinned_item"
            // Integration / reminder / EKM system notices
            | "bot_add"
            | "bot_remove"
            | "reminder_add"
            | "tombstone"
            | "ekm_access_denied"
    )
}

/// Result of parsing a Slack webhook event.
pub enum SlackWebhookResult {
    /// URL verification challenge — return the challenge string immediately.
    Challenge(String),
    /// Parsed message ready for routing.
    Message(Box<InboundMessage>),
    /// Event should be ignored. The carried `reason` is a fixed-vocabulary
    /// label that gets emitted as a Prometheus label by the HTTP handler.
    Ignore(&'static str),
}

/// Slack adapter.
pub struct SlackAdapter {
    api: SlackApi,
    signing_secret: String,
    bot_user_id: Option<String>,
    /// Deduplication cache: event_id → () with 5-minute TTL.
    dedup_cache: Cache<String, ()>,
    /// Workspace (team) id memoized after the first `auth.test` call so
    /// `post_to_destination` doesn't ping Slack on every send.
    workspace_id: OnceCell<String>,
}

impl SlackAdapter {
    pub fn new(bot_token: &str, signing_secret: &str, bot_user_id: Option<String>) -> Self {
        Self {
            api: SlackApi::new(bot_token),
            signing_secret: signing_secret.to_string(),
            bot_user_id,
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_mins(5))
                .max_capacity(10_000)
                .build(),
            workspace_id: OnceCell::new(),
        }
    }

    /// Return the Slack workspace (team) ID, calling `auth.test` once and
    /// memoizing the result for the lifetime of the provider.
    async fn ensure_workspace_id(&self) -> Result<String, AdapterError> {
        self.workspace_id
            .get_or_try_init(|| async {
                let info = self.api.auth_test().await?;
                info.team_id.ok_or_else(|| {
                    AdapterError::AdapterApi("Slack auth.test returned no team_id".into())
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
    pub async fn auth_test(&self) -> Result<super::api::AuthTestInfo, AdapterError> {
        self.api.auth_test().await
    }

    /// List Slack channels the bot is a member of.
    pub async fn list_channels(&self) -> Result<Vec<super::api::SlackChannelInfo>, AdapterError> {
        self.api.list_conversations().await
    }

    /// Open a DM channel with a Slack user and return the external target ID.
    pub async fn open_dm(&self, slack_user_id: &str) -> Result<String, AdapterError> {
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
    ) -> Result<Option<String>, AdapterError> {
        self.api.post_message(channel, text, thread_ts).await
    }

    /// Resolve a Slack channel ID to its human-readable name (best-effort).
    pub async fn get_channel_name(&self, channel_id: &str) -> Result<Option<String>, AdapterError> {
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
    ) -> Result<Option<String>, AdapterError> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"SELECT u.id FROM users u
               JOIN user_provider_ids p ON u.id = p.user_id
               WHERE p.provider = 'slack' AND p.provider_user_id = $1
               LIMIT 1"#,
        )
        .bind(slack_user_id)
        .fetch_optional(db)
        .await
        .map_err(|e| AdapterError::Internal(format!("Slack provider ID lookup failed: {e}")))?;

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

    /// Look up a local system user by email. Shared by the `Adapter`
    /// trait impl and by `parse_event` (which also inspects bot flags).
    async fn resolve_user_by_email(
        &self,
        identity: &str,
        email: &str,
        db: &DbPool,
    ) -> Result<Option<String>, AdapterError> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT id, login FROM users WHERE email = $1 LIMIT 1")
                .bind(email)
                .fetch_optional(db)
                .await
                .map_err(|e| AdapterError::Internal(format!("User lookup failed: {e}")))?;

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
        config: &LoopContext,
        db: &DbPool,
        is_tracked_thread: bool,
    ) -> Result<SlackWebhookResult, AdapterError> {
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
            .map_err(|e| AdapterError::BadRequest(format!("Invalid JSON: {e}")))?;

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
            debug!(reason = "no_event_field", "Slack event ignored");
            return Ok(SlackWebhookResult::Ignore("no_event"));
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
                debug!(
                    reason = "duplicate_event_id",
                    event_id = %event_id,
                    event_type = %event_type,
                    "Slack event ignored"
                );
                return Ok(SlackWebhookResult::Ignore("duplicate"));
            }
            self.dedup_cache.insert(event_id.to_string(), ()).await;
        }

        // 6. Extract user + channel early (needed for bot filter, DM detection, and access policy)
        let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
        let channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");

        // 7. Ignore bot's own messages
        if let Some(ref bot_id) = self.bot_user_id {
            if user == bot_id {
                debug!(
                    reason = "user_is_own_bot",
                    event_type = %event_type,
                    user = %user,
                    source_id = %channel,
                    "Slack event ignored"
                );
                return Ok(SlackWebhookResult::Ignore("own_bot"));
            }
        }

        // 7b. Ignore Slack system messages (channel join/leave, topic/purpose
        // changes, edits, pins, …). These arrive as `message` events with a
        // `subtype` and are not real user input — without this, `all_messages`
        // mode (and the DM / tracked-thread bypasses below) spawns a task for
        // every "X has joined the channel" notification.
        if event_type == "message" && is_ignored_message_subtype(event_subtype) {
            debug!(
                reason = "system_subtype",
                event_type = %event_type,
                event_subtype = %event_subtype,
                user = %user,
                source_id = %channel,
                "Slack event ignored"
            );
            return Ok(SlackWebhookResult::Ignore("system_subtype"));
        }

        // 8. Parse typed provider config (needed for the bot filter and trigger filter)
        let slack_config = SlackAdapterConfig::from_config(config);

        // Also check for bot_id field (different from user field).
        // When `allow_bot_messages` is on, foreign-bot messages (e.g. a CI notifier)
        // are allowed through; the agent's own posts are still suppressed at the
        // binding layer via self_echo (message ts match), so this is not a loop.
        let mut is_bot_message = false;
        if let Some(bot_id_value) = event.get("bot_id") {
            if !slack_config.allow_bot_messages {
                debug!(
                    reason = "bot_id_field_present",
                    event_type = %event_type,
                    event_subtype = %event_subtype,
                    user = %user,
                    bot_id = %bot_id_value,
                    source_id = %channel,
                    "Slack event ignored"
                );
                return Ok(SlackWebhookResult::Ignore("bot_account"));
            }
            is_bot_message = true;
        }

        // DM channels (starting with "D") are always directed at the bot —
        // bypass trigger filter for them.
        let is_dm = channel.starts_with('D');

        match (slack_config.trigger, event_type) {
            (MessageTrigger::MentionOnly, "app_mention") => {} // process
            (MessageTrigger::AllMessages, "message") => {}     // process
            (MessageTrigger::AllMessages, "app_mention") => {} // also process mentions in all_messages mode
            // Always process thread replies to threads we're tracking (e.g. outbound-created)
            (_, "message") if is_tracked_thread => {
                debug!(event_id = %event_id, "Processing tracked thread reply (trigger bypass)");
            }
            // DMs are inherently directed at the bot — always process
            (_, "message") if is_dm => {
                debug!(event_id = %event_id, channel = %channel, "Processing DM (trigger bypass)");
            }
            _ => {
                debug!(
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
                return Ok(SlackWebhookResult::Ignore("trigger_filter"));
            }
        }

        // 9. Enforce access policy
        match enforce_slack_access(&slack_config, channel, user) {
            AccessDecision::Allowed => {}
            AccessDecision::Blocked | AccessDecision::PairingSent => {
                debug!(
                    reason = "access_policy",
                    event_type = %event_type,
                    user = %user,
                    source_id = %channel,
                    "Slack event ignored"
                );
                return Ok(SlackWebhookResult::Ignore("access_policy"));
            }
        }

        // 8. Extract message details
        let raw_text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let ts = event.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        let thread_ts = event.get("thread_ts").and_then(|v| v.as_str());

        if ts.is_empty() {
            debug!(
                reason = "empty_ts",
                event_type = %event_type,
                user = %user,
                source_id = %channel,
                "Slack event ignored"
            );
            return Ok(SlackWebhookResult::Ignore("empty_message"));
        }

        // 9. Detect bot mention BEFORE stripping (needed for selective reply logic)
        let is_bot_mentioned = event_type == "app_mention"
            || self
                .bot_user_id
                .as_ref()
                .is_some_and(|bid| raw_text.contains(&format!("<@{bid}>")));

        // Resolve the message text. The top-level `text` is empty for app/bot
        // posts that carry their content in Block Kit `blocks` or legacy
        // `attachments` (e.g. CI notifiers posting via incoming webhooks).
        // Mirror the thread-history precedence (blocks → attachments → text) so
        // such messages aren't dropped as empty. The block/attachment helpers
        // already run `format_slack_text` internally; the plain-text branch
        // formats explicitly. We deliberately do NOT strip the bot's own
        // `<@bot_id>` mention — the agent's system prompt explains that token.
        let clean_text = event
            .get("blocks")
            .and_then(|b| b.as_array())
            .map(|b| super::format::extract_text_from_blocks(b))
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                event
                    .get("attachments")
                    .and_then(|a| a.as_array())
                    .map(|a| super::format::extract_text_from_attachments(a))
                    .filter(|t| !t.trim().is_empty())
            })
            .unwrap_or_else(|| super::format::format_slack_text(raw_text));

        if clean_text.trim().is_empty() {
            debug!(
                reason = "empty_text_after_format",
                event_type = %event_type,
                raw_text_len = raw_text.len(),
                has_blocks = event.get("blocks").is_some(),
                has_attachments = event.get("attachments").is_some(),
                is_bot_mentioned,
                user = %user,
                source_id = %channel,
                "Slack event ignored"
            );
            return Ok(SlackWebhookResult::Ignore("empty_after_format"));
        }

        // 10. Fetch user info once (bot flags + email) and resolve Slack user → system user.
        let slack_user_info = self.api.get_user_info(user).await.ok().flatten();

        // Drop bot/app-account senders: their messages should not drive task flow,
        // and — critically — they share emails across deployments so the email-based
        // user resolution is unsafe for them. This catches external deployment bots
        // that slip past the earlier `bot_id` field filter.
        //
        // When `allow_bot_messages` is on we skip this drop AND treat the sender as a
        // bot for the remainder of the pipeline (`is_bot_message`), so we never resolve
        // it by email or persist a `user_provider_ids` row (bots share emails across
        // deployments — that resolution is unsafe for them).
        if let Some(ref info) = slack_user_info {
            if info.is_bot || info.is_app_user {
                if !slack_config.allow_bot_messages {
                    let is_own_bot = self.bot_user_id.as_deref() == Some(user);
                    debug!(
                        slack_user = %user,
                        source_id = %channel,
                        is_own_bot,
                        handle = %info.handle.as_deref().unwrap_or(""),
                        "Ignoring Slack message from bot/app account"
                    );
                    return Ok(SlackWebhookResult::Ignore("bot_account"));
                }
                is_bot_message = true;
            }
        }

        // Try provider ID lookup first (fast, avoids extra Slack API call), then email.
        // Never resolve a bot by email — bots share emails across deployments.
        let mapped_user = if is_bot_message {
            None
        } else {
            self.resolve_user_by_slack_id(user, db).await?.or({
                match slack_user_info.as_ref().and_then(|i| i.email.clone()) {
                    Some(ref email) => self.resolve_user_by_email(user, email, db).await?,
                    None => None,
                }
            })
        };

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
            None if is_bot_message => {
                // Foreign bot (e.g. CI notifier): attribute to trigger owner, no error reply.
                info!(
                    slack_user = %user,
                    source_id = %channel,
                    fallback_user = %config.effective_user_id(),
                    "Bot message attributed to trigger owner"
                );
                config.effective_user_id().to_string()
            }
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
                return Ok(SlackWebhookResult::Ignore("user_not_mapped"));
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
            provider: AdapterKind::Slack,
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
            external_event_id: Some(super::external_id::message_ts(ts)),
            source_task_id: None,
        })))
    }
}

/// Parse a Slack destination string into `(channel, optional thread_ts)`.
///
/// Accepted shapes:
/// - `channel:<id>` — top-level post to `<id>`
/// - `dm:<id>` — top-level post to DM channel `<id>` (`D…`)
/// - `thread:<channel>/<thread_ts>` — reply in existing thread
/// - raw `<id>` (no prefix) — treated as a channel ID; matches the legacy
///   behavior before the prefix syntax was introduced.
fn parse_slack_destination(destination: &str) -> Result<(String, Option<String>), AdapterError> {
    let raw = destination.trim();
    if raw.is_empty() {
        return Err(AdapterError::BadRequest(
            "Slack destination must not be empty".into(),
        ));
    }
    if let Some(rest) = raw.strip_prefix("thread:") {
        let (channel, thread_ts) = rest.split_once('/').ok_or_else(|| {
            AdapterError::BadRequest(
                "Slack thread destination must be `thread:<channel>/<thread_ts>`".into(),
            )
        })?;
        if channel.is_empty() || thread_ts.is_empty() {
            return Err(AdapterError::BadRequest(
                "Slack thread destination requires both channel and thread_ts".into(),
            ));
        }
        return Ok((channel.to_string(), Some(thread_ts.to_string())));
    }
    if let Some(rest) = raw.strip_prefix("channel:") {
        if rest.is_empty() {
            return Err(AdapterError::BadRequest(
                "Slack channel destination must not be empty".into(),
            ));
        }
        return Ok((rest.to_string(), None));
    }
    if let Some(rest) = raw.strip_prefix("dm:") {
        if rest.is_empty() {
            return Err(AdapterError::BadRequest(
                "Slack DM destination must not be empty".into(),
            ));
        }
        return Ok((rest.to_string(), None));
    }
    // Raw fallback — accept a bare channel ID for backward compatibility.
    Ok((raw.to_string(), None))
}

/// Strip leading/trailing `:` characters and validate that the result is a
/// non-empty Slack-compatible bare reaction name.
///
/// Slack expects bare names (`thumbsup`), not `:thumbsup:`. Returns
/// `BadRequest` for empty input so the caller surfaces a clear error before
/// hitting the API.
fn normalize_slack_emoji(emoji: &str) -> Result<&str, AdapterError> {
    let trimmed = emoji.trim().trim_matches(':');
    if trimmed.is_empty() {
        return Err(AdapterError::BadRequest("Empty emoji name".into()));
    }
    Ok(trimmed)
}

/// Resolve the `(channel, ts)` pair needed by Slack's reactions API from the
/// target message's external id, falling back to the binding's
/// `destination_config` for `channel`.
fn parse_slack_reaction_target<'a>(
    binding: &'a BindingContext,
    target_external_id: &'a str,
) -> Result<(String, &'a str), AdapterError> {
    let ts = super::external_id::ts_from_message_id(target_external_id).ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "Slack reaction target `{target_external_id}` is not a valid slack:ts:<ts> id"
        ))
    })?;
    let channel = binding
        .destination_config
        .get("channel")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            AdapterError::BadRequest(
                "Slack binding destination_config has no `channel` field".into(),
            )
        })?
        .to_string();
    Ok((channel, ts))
}

#[async_trait::async_trait]
impl Adapter for SlackAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Slack
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
    ) -> Result<BindingDestination, AdapterError> {
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
        rule: &SubscriptionMatch,
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
            .then(|| {
                "[You were @mentioned - a reply via `moat messages reply --message <id>` is required]"
                    .to_string()
            })
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
    ) -> Result<Option<String>, AdapterError> {
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
        _config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let channel = thread
            .destination_config
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AdapterError::Internal("No channel in thread properties".into()))?;
        let thread_ts = thread
            .destination_config
            .get("thread_ts")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AdapterError::Internal("No thread_ts in thread properties".into()))?;

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
        _connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let thread = crate::bindings::runtime::binding_context(binding);
        let config = rule.map_or_else(
            || LoopContext {
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
                routing_mode: crate::adapters::models::RoutingMode::BySubject,
                prompt_template: None,
                active: true,
                allow_direct_send: false,
                provider_config: None,
                ongoing_task_id: None,
                scope: moatless_backend_core::scope::Scope::Global,
                task_name: None,
                tag_ids: vec![],
                run_as_user_id: None,
                agent_type: "claude-code".to_string(),
                inactive_ttl_days: None,
                created_by: "system".to_string(),
                created_at: binding.created_at,
                updated_at: binding.last_event_at,
            },
            |r| crate::bindings::runtime::trigger_context(r, "slack"),
        );
        self.send_reply(&thread, &config, message).await
    }

    /// Slack has to ask its API for the DM channel that pairs the bot with the
    /// user before anything can be posted there.
    async fn dm_destination(&self, provider_user_id: &str) -> Result<Option<String>, AdapterError> {
        let dm_channel = self.open_dm(provider_user_id).await?;
        Ok(Some(format!("dm:{dm_channel}")))
    }

    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<DestinationPostResult, AdapterError> {
        let workspace_id = self.ensure_workspace_id().await?;
        let (channel, thread_ts) = parse_slack_destination(destination)?;

        let ts = self
            .api
            .post_message(&channel, &message.text, thread_ts.as_deref())
            .await?
            .ok_or_else(|| AdapterError::Internal("Slack did not return message ts".into()))?;

        // Files upload into the same thread (or top-level when there's no thread).
        let upload_thread_ts = thread_ts.clone().unwrap_or_else(|| ts.clone());
        for file in &message.files {
            if let Err(e) = self
                .api
                .upload_file(
                    &channel,
                    Some(&upload_thread_ts),
                    &file.filename,
                    file.data.clone(),
                )
                .await
            {
                warn!(
                    filename = %file.filename,
                    error = %e,
                    "Failed to upload file attachment to Slack"
                );
            }
        }

        // For top-level posts the binding's `thread_ts` IS the new `ts`.
        // For thread replies the binding's `thread_ts` is the ORIGINAL thread anchor
        // (so future replies thread under the same parent, not under our new message).
        let binding_thread_ts = thread_ts.clone().unwrap_or_else(|| ts.clone());

        Ok(DestinationPostResult {
            message_external_id: ts.clone(),
            subject_kind: "slack_thread",
            subject_key: format!("{workspace_id}:{channel}:{binding_thread_ts}"),
            destination_config: serde_json::json!({
                "channel": channel,
                "ts": ts,
                "thread_ts": binding_thread_ts,
            }),
        })
    }

    async fn send_message(
        &self,
        config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<NewThreadInfo, AdapterError> {
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
            .ok_or_else(|| AdapterError::Internal("Slack did not return message ts".into()))?;

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

    async fn add_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        let (channel, ts) = parse_slack_reaction_target(binding, target_external_id)?;
        let name = normalize_slack_emoji(emoji)?;
        self.api.add_reaction(&channel, name, ts).await?;
        Ok(None)
    }

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<(), AdapterError> {
        let (channel, ts) = parse_slack_reaction_target(binding, target_external_id)?;
        let name = normalize_slack_emoji(emoji)?;
        self.api.remove_reaction(&channel, name, ts).await
    }

    async fn fetch_context(
        &self,
        thread: &BindingContext,
        config: &LoopContext,
        limit: usize,
        exclude_external_ids: &[String],
    ) -> Result<Option<String>, AdapterError> {
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

        let slack_config = SlackAdapterConfig::from_config(config);
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
    use crate::adapters::models::{RoutingMode, SubscriptionMatch};
    use crate::adapters::Adapter;
    use moatless_backend_core::scope::Scope;

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
    fn system_message_subtypes_are_ignored() {
        // The reported incident: "X has joined the channel".
        assert!(is_ignored_message_subtype("channel_join"));
        // The "others like it" family the join belongs to.
        for subtype in [
            "channel_leave",
            "channel_topic",
            "channel_purpose",
            "channel_name",
            "channel_archive",
            "channel_unarchive",
            "group_join",
            "group_leave",
            "message_changed",
            "message_deleted",
            "pinned_item",
            "unpinned_item",
            "bot_add",
            "bot_remove",
        ] {
            assert!(
                is_ignored_message_subtype(subtype),
                "{subtype} should be ignored"
            );
        }
    }

    #[test]
    fn real_message_subtypes_are_not_ignored() {
        // A plain user message has no subtype.
        assert!(!is_ignored_message_subtype(""));
        // Subtypes that still carry genuine user input must flow through.
        for subtype in ["file_share", "thread_broadcast", "me_message"] {
            assert!(
                !is_ignored_message_subtype(subtype),
                "{subtype} carries user input and must not be ignored"
            );
        }
        // `bot_message` is gated separately (allow_bot_messages), not here.
        assert!(!is_ignored_message_subtype("bot_message"));
    }

    #[test]
    fn binding_destination_from_existing_thread_uses_workspace_channel_and_thread() {
        let provider = SlackAdapter::new("token", "secret", None);
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
        let provider = SlackAdapter::new("token", "secret", None);
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

    fn test_rule(provider: &str, source_matcher: &str) -> SubscriptionMatch {
        let now = chrono::Utc::now();
        SubscriptionMatch {
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
            inactive_ttl_days: None,
            allow_direct_send: true,
            provider_config: None,
            ongoing_task_id: None,
            close_task_on_close: true,
            notify_on_success: false,
            active: true,
            created_by: "user-1".into(),
            scope: Scope::Global,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}
