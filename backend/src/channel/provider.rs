//! ChannelProvider trait and ChannelProviders registry.

use std::collections::HashMap;
use std::sync::Arc;

use std::any::Any;

use super::models::{
    BindingContext, ProviderConnection, TaskBinding, TaskTriggerRoute, TriggerContext,
};
use crate::db::DbPool;

use super::error::ChannelError;
use super::models::{InboundMessage, NewThreadInfo, OutboundReply};

pub use moatless_api_schemas::enums::ChannelType;

/// Result of posting to a provider-specific destination by raw ID.
///
/// Returned by [`ChannelProvider::post_to_destination`] and used by callers to
/// build a matching [`TaskBinding`] row when the post is associated with
/// a task.
#[derive(Debug, Clone)]
pub struct DestinationPostResult {
    /// Provider-specific external message ID (e.g. Slack message `ts`).
    pub message_external_id: String,
    /// Provider-specific binding `subject_kind`.
    pub subject_kind: &'static str,
    /// Binding `subject_key` matching the format produced during inbound
    /// routing for this provider.
    pub subject_key: String,
    /// Binding `destination_config` JSON used by `send_reply_for_binding`.
    pub destination_config: serde_json::Value,
}

/// Provider-owned destination data for a task binding.
#[derive(Debug, Clone)]
pub struct BindingDestination {
    /// Binding `subject_kind` (for example, a provider-defined thread kind).
    pub subject_kind: &'static str,
    /// Stable provider-defined lookup key for the bound subject.
    pub subject_key: String,
    /// Provider-defined JSON needed to send future replies.
    pub destination_config: serde_json::Value,
    /// Optional cursor seeded when the binding is created from an outbound send.
    pub last_external_event_id: Option<String>,
}

impl BindingDestination {
    pub fn new(
        subject_kind: &'static str,
        subject_key: String,
        destination_config: serde_json::Value,
    ) -> Self {
        Self {
            subject_kind,
            subject_key,
            destination_config,
            last_external_event_id: None,
        }
    }

    pub fn with_last_external_event_id(mut self, external_id: impl Into<String>) -> Self {
        self.last_external_event_id = Some(external_id.into());
        self
    }
}

/// Channel provider handles outbound messaging, user resolution, and context fetching.
///
/// Inbound message handling is NOT part of this trait — each provider implements
/// its own inbound transport (webhook handler, WebSocket consumer, poll loop,
/// etc.) that produces `InboundMessage` for the shared routing pipeline.
#[async_trait::async_trait]
pub trait ChannelProvider: Send + Sync {
    /// Channel type identifier.
    fn channel_type(&self) -> ChannelType;

    /// Canonical provider key for channel messages and provider-facing labels.
    fn provider_key(&self) -> &'static str {
        self.channel_type().as_str()
    }

    /// Provider key used by `provider_connections` and `task_triggers`.
    ///
    /// Most providers store the same key everywhere. GitHub PR uses the
    /// user-facing integration key (`github`) for connections/triggers while
    /// task bindings and message metadata use the channel key (`github_pr`).
    fn connection_provider_key(&self) -> &'static str {
        self.provider_key()
    }

    /// Scope value used to find the provider connection for a thread binding.
    fn connection_scope_value_from_existing_thread(
        &self,
        _source_id: &str,
        _thread_properties: &serde_json::Value,
    ) -> Result<String, ChannelError> {
        Ok("default".to_string())
    }

    /// Whether connection lookup should fall back to the `"default"` scope.
    fn allow_default_connection_fallback(&self) -> bool {
        true
    }

    /// Source matcher used to find the trigger route for a thread binding.
    fn trigger_source_matcher_from_existing_thread(
        &self,
        source_id: &str,
        _thread_properties: &serde_json::Value,
    ) -> Result<String, ChannelError> {
        Ok(source_id.to_string())
    }

    /// Whether trigger lookup should fall back to a wildcard source matcher.
    fn allow_wildcard_trigger_fallback(&self) -> bool {
        true
    }

    /// Whether this provider supports file attachments in outbound messages.
    ///
    /// Providers that return `false` will cause an error when files are
    /// included in an outbound destinationing them.
    fn supports_files(&self) -> bool {
        false
    }

    /// Maximum length (in characters) for an outbound reply's text body.
    ///
    /// Replies longer than this are truncated with an ellipsis before being
    /// handed to the provider. Default 4000 suits chat surfaces (Slack,
    /// Telegram); providers like GitHub that allow much longer comments
    /// should override this.
    fn max_message_length(&self) -> usize {
        4000
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
        thread: &BindingContext,
        config: &TriggerContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError>;

    /// Send a reply back to the subject identified by a `TaskBinding`.
    ///
    /// The default implementation adapts the canonical binding into the
    /// provider-facing config/thread context.
    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        _connection: &ProviderConnection,
        rule: Option<&TaskTriggerRoute>,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        let Some(rule) = rule else {
            return Err(ChannelError::BadRequest(
                "Provider needs a trigger-backed binding for outbound replies".into(),
            ));
        };
        let config = super::binding_runtime::trigger_context(rule, self.provider_key());
        let thread = super::binding_runtime::binding_context(binding);
        self.send_reply(&thread, &config, message).await
    }

    /// Post to a provider-specific destination by raw ID.
    ///
    /// Used for ad-hoc sends and to bootstrap a `task_binding` without a
    /// trigger. The returned [`DestinationPostResult`] carries everything the
    /// caller needs to persist a matching binding row.
    ///
    /// Default: returns a `BadRequest` so providers that have no notion of an
    /// addressable raw destination (cron, webhook, GitHub PR) surface a clear
    /// error rather than silently ignoring the call.
    async fn post_to_destination(
        &self,
        _destination: &str,
        _message: &OutboundReply,
    ) -> Result<DestinationPostResult, ChannelError> {
        Err(ChannelError::BadRequest(format!(
            "{:?} provider does not support direct destination sends",
            self.channel_type()
        )))
    }

    /// Send a new top-level message to a channel (no existing thread).
    ///
    /// Returns [`NewThreadInfo`] with the external thread ID and properties
    /// needed to create a [`BindingContext`] for future replies.
    ///
    /// Default: returns an error. Providers that support outbound-initiated
    /// messages (e.g., Slack) must override this.
    async fn send_message(
        &self,
        _config: &TriggerContext,
        _message: &OutboundReply,
    ) -> Result<NewThreadInfo, ChannelError> {
        Err(ChannelError::Internal(format!(
            "{:?} does not support send_message",
            self.channel_type()
        )))
    }

    /// Build the binding destination for an existing external thread.
    fn binding_destination_from_existing_thread(
        &self,
        _source_id: &str,
        _external_thread_id: &str,
        _thread_properties: &serde_json::Value,
        _filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, ChannelError> {
        Err(ChannelError::BadRequest(format!(
            "{} provider does not support task bindings",
            self.provider_key()
        )))
    }

    /// Build the binding destination for a newly-created external thread.
    fn binding_destination_from_new_thread(
        &self,
        _rule: &TaskTriggerRoute,
        _new_thread_info: &NewThreadInfo,
    ) -> Result<BindingDestination, ChannelError> {
        Err(ChannelError::BadRequest(format!(
            "{} provider does not support rules-based direct send",
            self.provider_key()
        )))
    }

    /// Expand a channel external id into all ids that should be considered
    /// equivalent for idempotency checks.
    fn external_id_aliases(&self, external_id: &str) -> Vec<String> {
        vec![external_id.to_string()]
    }

    /// Optional provider-specific suffix for sender labels delivered to agents.
    fn sender_label_suffix(&self, _msg: &InboundMessage) -> Option<String> {
        None
    }

    /// Optional provider-specific prefix for inbound messages delivered to agents.
    fn inbound_message_prefix(&self, _msg: &InboundMessage) -> Option<String> {
        None
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
        _thread: &BindingContext,
        _config: &TriggerContext,
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
            let bot_user_id =
                resolve_slack_bot_user_id(token, config.slack.bot_user_id.as_deref()).await;
            let slack = super::slack::provider::SlackProvider::new(token, secret, bot_user_id);
            providers.register(Arc::new(slack));
            tracing::info!("Slack channel provider configured");
        }

        // Webhook provider is always available (config is per-channel in provider_config JSONB)
        providers.register(Arc::new(super::webhook::provider::WebhookProvider::new()));
        tracing::info!("Webhook channel provider configured");

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
                db.clone(),
            );
            providers.register(Arc::new(github_pr));
            tracing::info!("GitHub PR channel provider configured");
        }

        // Linear provider (always available — per-team auth resolves via
        // provider_tokens; env-level token is the fallback)
        {
            let linear = super::linear::provider::LinearProvider::new(
                config.linear.webhook_secret.clone(),
                config.linear.api_token.clone(),
                db,
            );
            providers.register(Arc::new(linear));
            tracing::info!("Linear channel provider configured");
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
        if let Some(provider) = ChannelType::parse(name).and_then(|ct| self.providers.get(&ct)) {
            return Some(provider);
        }
        self.providers.values().find(|provider| {
            provider.provider_key() == name || provider.connection_provider_key() == name
        })
    }

    pub fn provider_key_for_name(&self, name: &str) -> Option<&'static str> {
        self.get_by_name(name)
            .map(|provider| provider.provider_key())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::models::OutboundReply;
    use crate::db::DbPool;

    struct DummyProvider;

    #[async_trait::async_trait]
    impl ChannelProvider for DummyProvider {
        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        async fn resolve_user(
            &self,
            _identity: &str,
            _db: &DbPool,
        ) -> Result<Option<String>, ChannelError> {
            Ok(None)
        }

        async fn send_reply(
            &self,
            _thread: &super::super::models::BindingContext,
            _config: &super::super::models::TriggerContext,
            _message: &OutboundReply,
        ) -> Result<Option<String>, ChannelError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn default_post_to_destination_returns_bad_request() {
        let provider = DummyProvider;
        let reply = OutboundReply {
            text: "hi".into(),
            task_url: None,
            is_error: false,
            files: vec![],
        };
        let err = provider
            .post_to_destination("chat-123", &reply)
            .await
            .expect_err("default impl should error");
        match err {
            ChannelError::BadRequest(msg) => {
                assert!(
                    msg.contains("does not support direct destination sends"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected BadRequest, got: {other:?}"),
        }
    }
}

/// Resolve the bot's Slack user ID.
///
/// Prefers the explicit env-var value; when absent, calls `auth.test` so the
/// agent's system prompt always has the bot's identity available.
async fn resolve_slack_bot_user_id(bot_token: &str, configured: Option<&str>) -> Option<String> {
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
