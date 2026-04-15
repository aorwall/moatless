//! ChannelProvider trait and ChannelProviders registry.

use std::collections::HashMap;
use std::sync::Arc;

use std::any::Any;

use super::models::{ChannelConfig, ChannelThread};
use crate::db::DbPool;

use super::error::ChannelError;
use super::models::{InboundMessage, NewThreadInfo, OutboundReply};

pub use moatless_api_schemas::enums::ChannelType;

/// Channel provider handles outbound messaging, user resolution, and context fetching.
///
/// Inbound message handling is NOT part of this trait — each provider implements
/// its own ingress (webhook handler, WebSocket consumer, poll loop, etc.)
/// that produces `InboundMessage` for the shared routing pipeline.
#[async_trait::async_trait]
pub trait ChannelProvider: Send + Sync {
    /// Channel type identifier.
    fn channel_type(&self) -> ChannelType;

    /// Whether this provider supports file attachments in outbound messages.
    ///
    /// Providers that return `false` will cause an error when files are
    /// included in an outbound reply targeting them.
    fn supports_files(&self) -> bool {
        false
    }

    /// Downcast support for provider-specific operations.
    fn as_any(&self) -> &dyn Any;

    /// Resolve an external identity (Slack user ID, email address) to a system user ID.
    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, ChannelError>;

    /// Send a reply back to the channel thread.
    ///
    /// Returns the external message ID if the provider tracks outbound messages
    /// (e.g., Slack `ts`). Providers that don't track outbound return `Ok(None)`.
    async fn send_reply(
        &self,
        thread: &ChannelThread,
        config: &ChannelConfig,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError>;

    /// Send a new top-level message to a channel (no existing thread).
    ///
    /// Returns [`NewThreadInfo`] with the external thread ID and properties
    /// needed to create a [`ChannelThread`] for future replies.
    ///
    /// Default: returns an error. Providers that support outbound-initiated
    /// messages (e.g., Slack) must override this.
    async fn send_message(
        &self,
        _config: &ChannelConfig,
        _message: &OutboundReply,
    ) -> Result<NewThreadInfo, ChannelError> {
        Err(ChannelError::Internal(format!(
            "{:?} does not support send_message",
            self.channel_type()
        )))
    }

    /// Provider-specific reply instructions appended to the system prompt for
    /// channel-originated tasks.
    ///
    /// These are appended after the generic `CHANNEL_REPLY_INSTRUCTIONS` and the
    /// optional per-config `prompt_template`. Use this to document provider-specific
    /// features like mention syntax, file attachments, or formatting.
    ///
    /// Default: no provider-specific instructions.
    fn reply_instructions(&self) -> Option<String> {
        None
    }

    /// Optional prefix prepended to inbound thread-reply messages to hint to
    /// the agent that this is a follow-up where it may choose to stay silent.
    ///
    /// Called only when `msg.thread_id.is_some()`. Providers inspect
    /// `msg.provider_context` (e.g., Slack's `is_bot_mentioned`) to decide
    /// whether the hint applies.
    ///
    /// Default: `None` — no hint prepended.
    fn thread_reply_hint(&self, _msg: &InboundMessage) -> Option<String> {
        None
    }

    /// Fetch thread context for enriching agent messages (e.g., Slack thread replies).
    /// Returns formatted context string or None if not supported.
    ///
    /// `exclude_external_ids` contains external message IDs already delivered to the task.
    /// Providers should filter these out to avoid sending duplicate context.
    async fn fetch_context(
        &self,
        _thread: &ChannelThread,
        _config: &ChannelConfig,
        _limit: usize,
        _exclude_external_ids: &[String],
    ) -> Result<Option<String>, ChannelError> {
        Ok(None)
    }
}

/// Registry of channel providers keyed by ChannelType.
#[derive(Clone)]
pub struct ChannelProviders {
    providers: HashMap<ChannelType, Arc<dyn ChannelProvider>>,
}

impl Default for ChannelProviders {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelProviders {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Build channel providers from application config.
    ///
    /// Returns `None` when no providers are configured.
    pub async fn from_config(config: &crate::config::Config, db: DbPool) -> Option<Self> {
        let mut providers = Self::new();

        if let (Some(ref token), Some(ref secret)) =
            (&config.slack.bot_token, &config.slack.signing_secret)
        {
            let bot_user_id = resolve_slack_bot_user_id(
                token,
                config.slack.bot_user_id.as_deref(),
            )
            .await;
            let slack = super::slack::provider::SlackProvider::new(token, secret, bot_user_id);
            providers.register(Arc::new(slack));
            tracing::info!("Slack channel provider configured");
        }

        // Webhook provider is always available (config is per-channel in provider_config JSONB)
        providers.register(Arc::new(super::webhook::provider::WebhookProvider::new()));
        tracing::info!("Webhook channel provider configured");

        // Cron provider is always available (schedule config is per-channel in provider_config JSONB)
        providers.register(Arc::new(super::cron::provider::CronProvider::new()));
        tracing::info!("Cron channel provider configured");

        if let Some(ref token) = config.telegram.bot_token {
            let telegram = super::telegram::provider::TelegramProvider::new(token);
            providers.register(Arc::new(telegram));
            tracing::info!("Telegram channel provider configured");
        }

        // GitHub PR provider (always available — config is per-channel in provider_config JSONB)
        {
            let github_pr = super::github_pr::provider::GithubPrProvider::new(
                config.github_pr.webhook_secret.clone(),
                config.github_pr.bot_token.clone(),
                db,
            );
            providers.register(Arc::new(github_pr));
            tracing::info!("GitHub PR channel provider configured");
        }

        Some(providers)
    }

    pub fn register(&mut self, provider: Arc<dyn ChannelProvider>) {
        self.providers.insert(provider.channel_type(), provider);
    }

    pub fn get(&self, channel_type: ChannelType) -> Option<&Arc<dyn ChannelProvider>> {
        self.providers.get(&channel_type)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&Arc<dyn ChannelProvider>> {
        ChannelType::parse(name).and_then(|ct| self.providers.get(&ct))
    }

    /// Returns user-facing provider types (excludes internal-only providers like Webhook and Cron).
    pub fn registered_types(&self) -> Vec<ChannelType> {
        self.providers
            .keys()
            .filter(|ct| !matches!(ct, ChannelType::Webhook | ChannelType::Cron))
            .copied()
            .collect()
    }
}

/// Resolve the bot's Slack user ID.
///
/// Prefers the explicit env-var value; when absent, calls `auth.test` so the
/// agent's system prompt always has the bot's identity available.
async fn resolve_slack_bot_user_id(
    bot_token: &str,
    configured: Option<&str>,
) -> Option<String> {
    if let Some(id) = configured {
        let id = id.trim();
        if !id.is_empty() {
            tracing::info!(bot_user_id = %id, "Slack bot identity from env var");
            return Some(id.to_string());
        }
    }

    let api = super::slack::api::SlackApi::new(bot_token);
    match api.auth_test().await {
        Ok(info) => {
            tracing::info!(
                bot_user_id = %info.user_id,
                handle = %info.user.as_deref().unwrap_or(""),
                team = %info.team.as_deref().unwrap_or(""),
                "Slack bot identity resolved via auth.test"
            );
            Some(info.user_id)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Could not resolve Slack bot identity — set SLACK_BOT_USER_ID explicitly; \
                 agent system prompt will fall back to an unknown-identity message"
            );
            None
        }
    }
}

