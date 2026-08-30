//! Turning one Slack Events API payload into either an [`InboundMessage`] or a
//! reason it was dropped.
//!
//! Normalization decides; it does not act. The sender lookup is the only I/O it
//! performs — the notice an unmapped sender gets and the provider-id row a
//! mapped one writes come back as [`Effect`]s for the caller to apply, so the
//! decision is a value that can be asserted without a Slack workspace or a
//! database behind it.

use tracing::{debug, info, warn};

use moatless_loops::adapters::error::AdapterError;
use moatless_adapter_model::models::{AccessDecision, MessageTrigger};
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::LoopContext;
use moatless_types::adapter::AdapterKind;

use moatless_backend_core::identity::ProviderIdentityLink;

use super::access::enforce_slack_access;
use super::api::SlackApi;
use super::auth;
use super::config::SlackAdapterConfig;

/// What normalization needs to know about the delivery it is looking at.
pub(super) struct NormalizeContext<'a> {
    pub(super) config: &'a LoopContext,
    pub(super) api: &'a SlackApi,
    pub(super) users: &'a dyn ProviderIdentityLink,
    pub(super) bot_user_id: Option<&'a str>,
    /// True when the event is a reply in a thread a Binding already tracks,
    /// which bypasses the trigger filter.
    pub(super) is_tracked_thread: bool,
}

/// The outcome of normalizing one payload.
pub(super) enum Normalization {
    Message {
        message: Box<InboundMessage>,
        effects: Vec<Effect>,
    },
    /// The carried `reason` is a fixed-vocabulary label that gets emitted as a
    /// Prometheus label by the HTTP handler.
    Ignored {
        reason: &'static str,
        effects: Vec<Effect>,
    },
}

/// Something normalization decided to do but did not do itself.
pub(super) enum Effect {
    /// The notice an unmapped top-level sender gets, in their own thread.
    PostNotice {
        channel: String,
        thread_ts: String,
        text: String,
    },
    /// Remember a resolved sender, so the next delivery from them costs no
    /// email lookup.
    CacheProviderId {
        user_id: uuid::Uuid,
        slack_user_id: String,
    },
}

/// Apply what normalization decided, best-effort and in order: neither effect
/// can fail a delivery, and a re-delivery repeats both exactly as it repeats
/// the normalization that produced them.
pub(super) async fn apply_effects(
    api: &SlackApi,
    users: &dyn ProviderIdentityLink,
    effects: &[Effect],
) {
    for effect in effects {
        match effect {
            Effect::PostNotice {
                channel,
                thread_ts,
                text,
            } => {
                let _ = api.post_message(channel, text, Some(thread_ts)).await;
            }
            Effect::CacheProviderId {
                user_id,
                slack_user_id,
            } => auth::cache_provider_id(users, *user_id, slack_user_id).await,
        }
    }
}

/// The fields of a Slack event the Adapter reads, extracted once.
struct SlackEvent<'a> {
    body: &'a serde_json::Value,
    event_type: &'a str,
    subtype: &'a str,
    /// The envelope's `event_id`, for logging only — deduplication is durable
    /// and keyed on the same value at admission.
    event_id: &'a str,
    user: &'a str,
    channel: &'a str,
    raw_text: &'a str,
    ts: &'a str,
    thread_ts: Option<&'a str>,
}

impl<'a> SlackEvent<'a> {
    fn from_payload(payload: &'a serde_json::Value) -> Option<Self> {
        let body = payload.get("event")?;
        let field = |name| body.get(name).and_then(serde_json::Value::as_str);
        Some(Self {
            body,
            event_type: field("type").unwrap_or(""),
            subtype: field("subtype").unwrap_or(""),
            event_id: payload
                .get("event_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            user: field("user").unwrap_or(""),
            channel: field("channel").unwrap_or(""),
            raw_text: field("text").unwrap_or(""),
            ts: field("ts").unwrap_or(""),
            thread_ts: field("thread_ts"),
        })
    }

    /// The writer's own Slack id, where the event names one.
    ///
    /// A bot post carries its identity in `bot_id` and leaves `user` absent,
    /// which `from_payload` reads as `""`. Persisting that would label the
    /// writer `slack: ` and name nobody, so it resolves to no id at all and the
    /// header falls back to the handle or is omitted.
    fn sender_external_id(&self) -> Option<String> {
        (!self.user.is_empty()).then(|| self.user.to_string())
    }

    /// Resolve the message text. The top-level `text` is empty for app/bot
    /// posts that carry their content in Block Kit `blocks` or legacy
    /// `attachments` (e.g. CI notifiers posting via incoming webhooks).
    /// Mirror the thread-history precedence (blocks → attachments → text) so
    /// such messages aren't dropped as empty. The block/attachment helpers
    /// already run `format_slack_text` internally; the plain-text branch
    /// formats explicitly. We deliberately do NOT strip the bot's own
    /// `<@bot_id>` mention — the agent's system prompt explains that token.
    fn clean_text(&self) -> String {
        self.body
            .get("blocks")
            .and_then(|b| b.as_array())
            .map(|b| super::format::extract_text_from_blocks(b))
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                self.body
                    .get("attachments")
                    .and_then(|a| a.as_array())
                    .map(|a| super::format::extract_text_from_attachments(a))
                    .filter(|t| !t.trim().is_empty())
            })
            .unwrap_or_else(|| super::format::format_slack_text(self.raw_text))
    }
}

/// Slack `message` events carry a `subtype` for system notifications — channel
/// join/leave, topic/purpose/name changes, archives, pins, edits, deletes, … —
/// rather than genuine user input. These must never be treated as task loops:
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

/// Every drop decided from the payload alone, before a single Slack call.
///
/// `Ok(is_bot_message)` when the event survives — true when the `bot_id` field
/// marked it as a foreign bot post that `allow_bot_messages` let through.
fn screen(
    event: &SlackEvent<'_>,
    slack_config: &SlackAdapterConfig,
    ctx: &NormalizeContext<'_>,
) -> Result<bool, &'static str> {
    // Ignore the bot's own messages.
    if ctx.bot_user_id == Some(event.user) {
        debug!(
            reason = "user_is_own_bot",
            event_type = %event.event_type,
            user = %event.user,
            source_id = %event.channel,
            "Slack event ignored"
        );
        return Err("own_bot");
    }

    // Ignore Slack system messages (channel join/leave, topic/purpose changes,
    // edits, pins, …). These arrive as `message` events with a `subtype` and
    // are not real user input — without this, `all_messages` mode (and the DM /
    // tracked-thread bypasses below) spawns a task for every "X has joined the
    // channel" notification.
    if event.event_type == "message" && is_ignored_message_subtype(event.subtype) {
        debug!(
            reason = "system_subtype",
            event_type = %event.event_type,
            event_subtype = %event.subtype,
            user = %event.user,
            source_id = %event.channel,
            "Slack event ignored"
        );
        return Err("system_subtype");
    }

    // Also check for bot_id field (different from user field).
    // When `allow_bot_messages` is on, foreign-bot messages (e.g. a CI notifier)
    // are allowed through; the agent's own posts are still suppressed at the
    // binding layer via self_echo (message ts match), so this is not a loop.
    let mut is_bot_message = false;
    if let Some(bot_id_value) = event.body.get("bot_id") {
        if !slack_config.allow_bot_messages {
            debug!(
                reason = "bot_id_field_present",
                event_type = %event.event_type,
                event_subtype = %event.subtype,
                user = %event.user,
                bot_id = %bot_id_value,
                source_id = %event.channel,
                "Slack event ignored"
            );
            return Err("bot_account");
        }
        is_bot_message = true;
    }

    // DM channels (starting with "D") are always directed at the bot —
    // bypass trigger filter for them.
    let is_dm = event.channel.starts_with('D');

    match (slack_config.trigger, event.event_type) {
        (MessageTrigger::MentionOnly, "app_mention") => {} // process
        (MessageTrigger::AllMessages, "message") => {}     // process
        (MessageTrigger::AllMessages, "app_mention") => {} // also process mentions in all_messages mode
        // Always process thread replies to threads we're tracking (e.g. outbound-created)
        (_, "message") if ctx.is_tracked_thread => {
            debug!(event_id = %event.event_id, "Processing tracked thread reply (trigger bypass)");
        }
        // DMs are inherently directed at the bot — always process
        (_, "message") if is_dm => {
            debug!(event_id = %event.event_id, channel = %event.channel, "Processing DM (trigger bypass)");
        }
        _ => {
            debug!(
                reason = "trigger_filter",
                event_type = %event.event_type,
                event_subtype = %event.subtype,
                trigger = ?slack_config.trigger,
                is_tracked_thread = ctx.is_tracked_thread,
                is_dm,
                user = %event.user,
                source_id = %event.channel,
                "Slack event ignored"
            );
            return Err("trigger_filter");
        }
    }

    match enforce_slack_access(slack_config, event.channel, event.user) {
        AccessDecision::Allowed => {}
        AccessDecision::Blocked | AccessDecision::PairingSent => {
            debug!(
                reason = "access_policy",
                event_type = %event.event_type,
                user = %event.user,
                source_id = %event.channel,
                "Slack event ignored"
            );
            return Err("access_policy");
        }
    }

    Ok(is_bot_message)
}

/// Put a name to every Slack id the message text still mentions.
///
/// Each id resolves to the Moatless login it is linked to, else to the handle
/// Slack knows it by, else stays an id. The bot's own id is deliberately left
/// alone: its system prompt tells it that `<@its_id>` is how it is addressed,
/// and renaming it there would break that.
async fn name_mentioned_users(
    text: &str,
    ctx: &NormalizeContext<'_>,
) -> String {
    let ids = super::format::mentioned_user_ids(text);
    if ids.is_empty() {
        return text.to_string();
    }

    let mut names = std::collections::HashMap::new();
    for id in ids {
        if ctx.bot_user_id == Some(id.as_str()) {
            continue;
        }
        match ctx.users.user_at(super::delivery::ADAPTER, &id).await {
            Ok(Some(user)) => {
                names.insert(id, user.login);
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                warn!(slack_user = %id, error = %e, "Mention lookup failed, leaving the raw id");
                continue;
            }
        }
        if let Ok(Some(info)) = ctx.api.get_user_info(&id).await {
            if let Some(handle) = info.handle {
                names.insert(id, handle);
            }
        }
    }

    super::format::name_mentions(text, &names)
}

/// The notice an unmapped top-level sender is answered with.
fn unmapped_sender_notice(email: Option<&str>) -> String {
    match email {
        Some(email) => format!(
            "Sorry, I couldn't find your account. Please make sure you're registered with the email address: {email}"
        ),
        None => "Sorry, I couldn't find your account. Please make sure you're registered in the system with the same email as your Slack profile.".to_string(),
    }
}

/// Normalize one Slack Events API payload.
///
/// Deduplication is not done here — it happens durably when the caller admits
/// the message, keyed on the envelope's `event_id`. Re-normalizing a
/// re-delivery is the price, and it is paid twice over: a repeat `users.info`
/// call, and a repeat notice if the sender is unmapped. It is still the right
/// trade — normalization is what decides whether there is any work to admit, so
/// declining to repeat it means acknowledging a delivery whose admission was
/// never attempted, which is how a webhook gets lost outright rather than
/// duplicated.
pub(super) async fn normalize(
    payload: &serde_json::Value,
    ctx: &NormalizeContext<'_>,
) -> Result<Normalization, AdapterError> {
    let ignored = |reason| {
        Ok(Normalization::Ignored {
            reason,
            effects: vec![],
        })
    };

    let Some(event) = SlackEvent::from_payload(payload) else {
        debug!(reason = "no_event_field", "Slack event ignored");
        return ignored("no_event");
    };

    let slack_config = SlackAdapterConfig::from_config(ctx.config);
    let mut is_bot_message = match screen(&event, &slack_config, ctx) {
        Ok(is_bot_message) => is_bot_message,
        Err(reason) => return ignored(reason),
    };

    if event.ts.is_empty() {
        debug!(
            reason = "empty_ts",
            event_type = %event.event_type,
            user = %event.user,
            source_id = %event.channel,
            "Slack event ignored"
        );
        return ignored("empty_message");
    }

    // Detect the mention BEFORE any rewriting (needed for selective reply logic).
    let is_bot_mentioned = event.event_type == "app_mention"
        || ctx
            .bot_user_id
            .is_some_and(|bid| event.raw_text.contains(&format!("<@{bid}>")));

    let clean_text = event.clean_text();
    if clean_text.trim().is_empty() {
        debug!(
            reason = "empty_text_after_format",
            event_type = %event.event_type,
            raw_text_len = event.raw_text.len(),
            has_blocks = event.body.get("blocks").is_some(),
            has_attachments = event.body.get("attachments").is_some(),
            is_bot_mentioned,
            user = %event.user,
            source_id = %event.channel,
            "Slack event ignored"
        );
        return ignored("empty_after_format");
    }

    // The sender lookup: one `users.info`, then the provider-id mapping before
    // the email one. A bot/app account either drops out here or — with
    // `allow_bot_messages` on — stays flagged as a bot for the rest of the way.
    let (mapped_user, sender_email, sender_handle) = match auth::resolve_inbound_sender(
        ctx.api,
        ctx.users,
        event.user,
        slack_config.allow_bot_messages,
        is_bot_message,
    )
    .await?
    {
        auth::SenderIdentity::Rejected { handle } => {
            debug!(
                slack_user = %event.user,
                source_id = %event.channel,
                is_own_bot = ctx.bot_user_id == Some(event.user),
                handle = %handle.as_deref().unwrap_or(""),
                "Ignoring Slack message from bot/app account"
            );
            return ignored("bot_account");
        }
        auth::SenderIdentity::Mapped { user_id, handle } => (Some(user_id), None, handle),
        auth::SenderIdentity::Unmapped {
            is_bot,
            email,
            handle,
        } => {
            is_bot_message = is_bot;
            (None, email, handle)
        }
    };

    let mut effects = Vec::new();
    if let Some(user_id) = mapped_user {
        effects.push(Effect::CacheProviderId {
            user_id,
            slack_user_id: event.user.to_string(),
        });
    }

    // The one place a resolved id becomes text: `InboundMessage` carries a
    // `String`, and the two fallbacks below are already one.
    let user_id = match mapped_user {
        Some(user_id) => user_id.to_string(),
        None if is_bot_message => {
            // Foreign bot (e.g. CI notifier): attribute to trigger owner, no error reply.
            info!(
                slack_user = %event.user,
                source_id = %event.channel,
                fallback_user = %ctx.config.effective_user_id(),
                "Bot message attributed to trigger owner"
            );
            ctx.config.effective_user_id().to_string()
        }
        None if event.thread_ts.is_some() => {
            // Thread reply from unmapped human — fall back to config owner
            // (user identity only matters for task creation)
            info!(
                slack_user = %event.user,
                source_id = %event.channel,
                fallback_user = %ctx.config.effective_user_id(),
                "Slack user not mapped, using config owner for thread reply"
            );
            ctx.config.effective_user_id().to_string()
        }
        None => {
            // Top-level message from unmapped human — block and notify.
            warn!(slack_user = %event.user, source_id = %event.channel, "Cannot process message: Slack user not mapped to system user");
            effects.push(Effect::PostNotice {
                channel: event.channel.to_string(),
                thread_ts: event.thread_ts.unwrap_or(event.ts).to_string(),
                text: unmapped_sender_notice(sender_email.as_deref()),
            });
            return Ok(Normalization::Ignored {
                reason: "user_not_mapped",
                effects,
            });
        }
    };

    // Last, so an event that never becomes a message spends no lookups on the
    // people it mentions.
    let clean_text = name_mentioned_users(&clean_text, ctx).await;

    let team_id = payload
        .get("team_id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    let thread_ts = event.thread_ts.unwrap_or(event.ts);

    Ok(Normalization::Message {
        message: Box::new(InboundMessage {
            adapter_kind: AdapterKind::Slack,
            source_id: event.channel.to_string(),
            thread_id: event.thread_ts.map(ToString::to_string),
            message_thread_id: thread_ts.to_string(),
            user_id,
            author: sender_handle
                .as_deref()
                .and_then(moatless_adapter_model::models::author_handle),
            // Always the writer's own Slack id, unlike `user_id` above, which
            // falls back to the trigger owner when the writer is unmapped.
            sender_external_id: event.sender_external_id(),
            text: clean_text,
            file_ids: vec![], // TODO: handle file attachments
            thread_properties: serde_json::json!({
                "channel": event.channel,
                "ts": event.ts,
                "thread_ts": thread_ts,
                "team_id": team_id,
            }),
            adapter_context: serde_json::json!({
                "team_id": team_id,
                "user_id": event.user,
                "is_bot_mentioned": is_bot_mentioned,
            }),
            external_event_id: Some(super::external_id::message_ts(event.ts)),
            source_task_id: None,
            // Slack makes no audience distinction: every event goes to every
            // task bound to the channel or thread, as it always has.
            audience: Audience::Everyone,
        }),
        effects,
    })
}

#[cfg(test)]
mod tests {
    use super::{SlackEvent, is_ignored_message_subtype};

    #[test]
    fn a_writer_with_a_slack_id_is_identified_by_it() {
        let payload = serde_json::json!({
            "event": { "type": "message", "user": "U0000000001", "text": "hi" }
        });
        let event = SlackEvent::from_payload(&payload).expect("an event");
        assert_eq!(event.sender_external_id().as_deref(), Some("U0000000001"));
    }

    /// A bot post allowed through by `allow_bot_messages` names itself in
    /// `bot_id`, not `user`. An empty id must not be stored as a real sender:
    /// it would render `<from>slack: </from>`, which names nobody.
    #[test]
    fn a_bot_post_with_no_user_field_has_no_sender_id() {
        let payload = serde_json::json!({
            "event": {
                "type": "message",
                "subtype": "bot_message",
                "bot_id": "B0DEPLOYBOT",
                "text": "deploy finished"
            }
        });
        let event = SlackEvent::from_payload(&payload).expect("an event");
        assert_eq!(event.user, "", "Slack omitted `user` entirely");
        assert_eq!(event.sender_external_id(), None);
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
}
