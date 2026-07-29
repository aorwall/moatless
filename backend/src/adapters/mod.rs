//! Adapter trait, registry, connection storage, and webhook routes.

pub mod common;
pub mod dao;
pub mod delivery;
pub mod error;
pub mod footer;
pub mod github_pr;
pub mod handlers;
pub(crate) mod inbound_delivery;
pub(crate) mod ingress;
pub mod linear;
pub mod metrics;
pub mod models;
pub mod routes;
pub mod signature;
pub mod slack;
pub(crate) mod subject_events;
pub mod telegram;
#[cfg(test)]
pub(crate) mod test_support;
pub mod webhook;

use std::collections::HashMap;
use std::sync::Arc;

use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::db::DbPool;
use crate::state::AppState;

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, ExternalStatus, InboundMessage, LoopContext, NewThreadInfo,
    OutboundReply, SubscriptionMatch, TaskBinding,
};

pub use moatless_api_schemas::enums::AdapterKind;

/// Result of posting to a provider-specific destination by raw ID.
///
/// Returned by [`Adapter::post_to_destination`] and used by callers to
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

/// Adapter handles outbound messaging, user resolution, and context fetching.
///
/// Inbound message handling is NOT part of this trait — each adapter implements
/// its own inbound transport (webhook handler, WebSocket consumer, poll loop,
/// etc.) that produces `InboundMessage` for the shared routing pipeline.
#[async_trait::async_trait]
pub trait Adapter: Send + Sync {
    /// Adapter type identifier.
    fn adapter_kind(&self) -> AdapterKind;

    /// Canonical provider key for external messages and provider-facing labels.
    fn provider_key(&self) -> &'static str {
        self.adapter_kind().as_str()
    }

    /// Adapter key used by adapter connections and trigger sources.
    ///
    /// Most adapters store the same key everywhere. GitHub PR uses the
    /// user-facing adapter key (`github`) for connections/triggers while
    /// task bindings and message metadata use the adapter target key (`github_pr`).
    fn connection_provider_key(&self) -> &'static str {
        self.provider_key()
    }

    /// Scope value used to find the provider connection for a thread binding.
    fn connection_scope_value_from_existing_thread(
        &self,
        _source_id: &str,
        _thread_properties: &serde_json::Value,
    ) -> Result<String, AdapterError> {
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
    ) -> Result<String, AdapterError> {
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

    /// Resolve an external identity (Slack user ID, email address) to a system user ID.
    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, AdapterError>;

    /// Send a reply back to the adapter target thread.
    ///
    /// Returns the external message ID if the provider tracks outbound messages
    /// (e.g., Slack `ts`). Providers that don't track outbound return `Ok(None)`.
    async fn send_reply(
        &self,
        thread: &BindingContext,
        config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError>;

    /// Send a reply back to the subject identified by a `TaskBinding`.
    ///
    /// The default implementation adapts the canonical binding into the
    /// provider-facing config/thread context.
    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        _connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let Some(rule) = rule else {
            return Err(AdapterError::BadRequest(
                "Provider needs a trigger-backed binding for outbound replies".into(),
            ));
        };
        let config = crate::bindings::runtime::trigger_context(rule, self.provider_key());
        let thread = crate::bindings::runtime::binding_context(binding);
        self.send_reply(&thread, &config, message).await
    }

    /// Reply specifically to the message identified by `target_external_id`.
    ///
    /// Default delegates to `send_reply_for_binding`: providers without a notion
    /// of per-message reply (Slack threads already collapse to the binding's
    /// thread; Linear has no per-comment threading) inherit the current behaviour.
    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        _target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        self.send_reply_for_binding(binding, connection, rule, message)
            .await
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
    ) -> Result<DestinationPostResult, AdapterError> {
        Err(AdapterError::BadRequest(format!(
            "{:?} provider does not support direct destination sends",
            self.adapter_kind()
        )))
    }

    /// Resolve the destination that addresses a single external user directly,
    /// given that user's provider-side id.
    ///
    /// The returned string is a [`Adapter::post_to_destination`] destination, so
    /// a provider that has to call its API to open the conversation (Slack) does
    /// it here, and one that can compute the destination outright (Telegram)
    /// simply formats it.
    ///
    /// Default: `Ok(None)` — this provider has no direct-message surface.
    async fn dm_destination(
        &self,
        _provider_user_id: &str,
    ) -> Result<Option<String>, AdapterError> {
        Ok(None)
    }

    /// Follow-up the provider runs after an outbound send created a binding.
    ///
    /// Called once the binding row exists, so a provider can link the moatless
    /// task back onto the external subject. Failures are the provider's to log
    /// and swallow — the send has already succeeded by this point.
    ///
    /// Default: does nothing.
    async fn on_binding_created(
        &self,
        _state: &AppState,
        _connection: &AdapterConnection,
        _binding: &TaskBinding,
        _task_id: &str,
    ) {
    }

    /// Send a new top-level message to an external destination (no existing thread).
    ///
    /// Returns [`NewThreadInfo`] with the external thread ID and properties
    /// needed to create a [`BindingContext`] for future replies.
    ///
    /// Default: returns an error. Providers that support outbound-initiated
    /// messages (e.g., Slack) must override this.
    async fn send_message(
        &self,
        _config: &LoopContext,
        _message: &OutboundReply,
    ) -> Result<NewThreadInfo, AdapterError> {
        Err(AdapterError::Internal(format!(
            "{:?} does not support send_message",
            self.adapter_kind()
        )))
    }

    /// Build the binding destination for an existing external thread.
    fn binding_destination_from_existing_thread(
        &self,
        _source_id: &str,
        _external_thread_id: &str,
        _thread_properties: &serde_json::Value,
        _filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, AdapterError> {
        Err(AdapterError::BadRequest(format!(
            "{} provider does not support task bindings",
            self.provider_key()
        )))
    }

    /// Build the binding destination for a newly-created external thread.
    fn binding_destination_from_new_thread(
        &self,
        _rule: &SubscriptionMatch,
        _new_thread_info: &NewThreadInfo,
    ) -> Result<BindingDestination, AdapterError> {
        Err(AdapterError::BadRequest(format!(
            "{} provider does not support rules-based direct send",
            self.provider_key()
        )))
    }

    /// Add a reaction (e.g. emoji) to an existing external message.
    ///
    /// `target_external_id` is the provider-namespaced id stored on
    /// `task_messages.external_event_id` (e.g. `slack:ts:…`,
    /// `github:ic:…`, `linear:comment:…`, `tg:m:…`).
    ///
    /// Returns the provider-specific reaction id when one is available
    /// (e.g. GitHub returns a numeric reaction id), otherwise `None`.
    ///
    /// Default: returns a `BadRequest` so providers that don't implement
    /// reactions surface a clear error.
    async fn add_reaction(
        &self,
        _binding: &BindingContext,
        _target_external_id: &str,
        _emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        Err(AdapterError::BadRequest(format!(
            "Provider {} does not support reactions",
            self.provider_key()
        )))
    }

    /// Remove a previously added reaction from an external message.
    ///
    /// Default: returns a `BadRequest` (see [`Adapter::add_reaction`]).
    async fn remove_reaction(
        &self,
        _binding: &BindingContext,
        _target_external_id: &str,
        _emoji: &str,
    ) -> Result<(), AdapterError> {
        Err(AdapterError::BadRequest(format!(
            "Provider {} does not support reactions",
            self.provider_key()
        )))
    }

    /// Expand an external event id into all ids that should be considered
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
    /// external-source tasks.
    ///
    /// Universal `moat messages reply --message <id>` guidance lives in the
    /// base sandbox-tools prompt (`backend/src/agent/prompt.rs`). Use this
    /// hook only to document provider-specific extras the base prompt can't
    /// know about — e.g. Slack mention syntax, file attachments, or formatting.
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

    /// Stable key naming the **external subject** a binding points at (the pull
    /// request, the issue), not the binding itself — so several tasks bound to
    /// the same subject share one cached status and one API call.
    ///
    /// Returned before any fetch happens, which is why it is a method of its
    /// own rather than a by-product of [`Adapter::fetch_binding_status`].
    ///
    /// Default: `None` — this provider has no live external status.
    fn binding_status_key(&self, _binding: &TaskBinding) -> Option<String> {
        None
    }

    /// Human label and web URL for the external subject a binding points at,
    /// for read paths that render a binding without knowing which provider it
    /// belongs to. Only the adapter knows the shape of its own
    /// `destination_config`, so only the adapter can read a label or a link out
    /// of it.
    ///
    /// Default: the binding's `subject_key` as the label and no URL.
    fn binding_label_and_url(&self, binding: &TaskBinding) -> (String, Option<String>) {
        (binding.subject_key.clone(), None)
    }

    /// Whether a normalized `external_state_kind` means the external subject is
    /// finished and can no longer change on its own (a merged PR, a completed
    /// issue).
    ///
    /// Default: `false` — nothing this provider reports is terminal.
    fn is_terminal_state_kind(&self, _state_kind: &str) -> bool {
        false
    }

    /// Fetch the live external status for a binding, calling the provider's API.
    ///
    /// `acting_user` is the owning task's creator, used by providers that
    /// resolve a per-user token. Returns `None` when no live status can be
    /// obtained (unsupported binding, no token, API error) — callers fall back
    /// to whatever is already persisted.
    ///
    /// Default: `None` — this provider has no live external status.
    async fn fetch_binding_status(
        &self,
        _binding: &TaskBinding,
        _acting_user: &str,
    ) -> Option<ExternalStatus> {
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
        _config: &LoopContext,
        _limit: usize,
        _exclude_external_ids: &[String],
    ) -> Result<Option<String>, AdapterError> {
        Ok(None)
    }
}

/// Registry of external adapters keyed by AdapterKind.
///
/// Alongside the trait-object map, the registry keeps a typed handle for each
/// adapter that has concrete-only methods, so code inside an adapter's own
/// module reaches it without downcasting.
///
/// For those four kinds the typed registrar is the only way in: it sets the
/// handle and inserts into the map in one call. The trait-object insert is
/// private to this module, so no caller can add one of them to the map and
/// leave its handle unset. Apart from those registrars, the only callers that
/// reach it — and so the only ones that insert without a handle — are
/// `register_webhook`, whose adapter has no concrete-only methods and hence no
/// handle, and the tests.
#[derive(Clone)]
pub struct Adapters {
    providers: HashMap<AdapterKind, Arc<dyn Adapter>>,
    github_pr: Option<Arc<self::github_pr::adapter::GithubPrAdapter>>,
    linear: Option<Arc<self::linear::adapter::LinearAdapter>>,
    slack: Option<Arc<self::slack::adapter::SlackAdapter>>,
    telegram: Option<Arc<self::telegram::adapter::TelegramAdapter>>,
}

impl Default for Adapters {
    fn default() -> Self {
        Self::new()
    }
}

impl Adapters {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            github_pr: None,
            linear: None,
            slack: None,
            telegram: None,
        }
    }

    /// Build adapters from application config.
    ///
    /// Returns `None` when no providers are configured.
    ///
    /// `github_apps` is the shared App registry — passed in so the
    /// `GithubPrAdapter` and `AppState` hit the same token cache. The
    /// `GithubPrAdapter` resolves runtime GitHub tokens through the canonical
    /// resolver, which needs the registry to mint installation tokens for bot
    /// users (`auth_method = 'github_app_installation'`).
    pub async fn from_config(
        config: &crate::config::Config,
        db: DbPool,
        github_apps: Arc<GitHubAppRegistry>,
    ) -> Option<Self> {
        use crate::auth::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
        use crate::auth::provider_tokens::{
            PgProviderTokenAuthRepository, ProviderTokenAuthRepository,
        };
        use crate::secret::audit::{PgSecretAuditRepository, SecretAuditRepository};
        use crate::secret::dao::{PgSecretRepository, SecretRepository};
        use crate::secret::service::{LocalEnvelopeSecretCipher, SecretService};

        let mut providers = Self::new();

        let provider_tokens: Arc<dyn ProviderTokenRepository> =
            Arc::new(PgProviderTokenRepository(db.clone()));
        let provider_token_auth: Arc<dyn ProviderTokenAuthRepository> =
            Arc::new(PgProviderTokenAuthRepository(db.clone()));
        let github_app_service = github_apps.default_service();

        // Needed so the GitHub adapter can resolve `secret_ref` provider tokens
        // (validated PATs live in `secrets`, not `provider_tokens`).
        let secret_records: Arc<dyn SecretRepository> = Arc::new(PgSecretRepository(db.clone()));
        let secret_audits: Arc<dyn SecretAuditRepository> =
            Arc::new(PgSecretAuditRepository(db.clone()));
        let secrets = Arc::new(SecretService::new(
            secret_records,
            secret_audits,
            LocalEnvelopeSecretCipher::from_config(&config.secrets),
        ));

        if let (Some(ref token), Some(ref secret)) =
            (&config.slack.bot_token, &config.slack.signing_secret)
        {
            let bot_user_id =
                resolve_slack_bot_user_id(token, config.slack.bot_user_id.as_deref()).await;
            let slack = self::slack::adapter::SlackAdapter::new(token, secret, bot_user_id);
            providers.register_slack(Arc::new(slack));
            tracing::info!("Slack adapter configured");
        }

        // Webhook adapter is always available (config is per trigger source).
        providers.register_webhook(Arc::new(self::webhook::adapter::WebhookAdapter::new()));
        tracing::info!("Webhook adapter configured");

        if let Some(ref token) = config.telegram.bot_token {
            let telegram = self::telegram::adapter::TelegramAdapter::new(token);
            providers.register_telegram(Arc::new(telegram));
            tracing::info!("Telegram adapter configured");
        }

        // GitHub PR adapter is always available; config is per trigger source.
        {
            let github_pr = self::github_pr::adapter::GithubPrAdapter::new(
                config.github_pr.webhook_secret.clone(),
                config.github_pr.bot_token.clone(),
                config.auth.frontend_url.clone(),
                db.clone(),
                provider_tokens.clone(),
                Some(provider_token_auth.clone()),
                github_app_service.clone(),
                github_apps.clone(),
            )
            .with_secrets(secrets.clone());
            providers.register_github_pr(Arc::new(github_pr));
            tracing::info!("GitHub PR adapter configured");
        }

        // Linear adapter — workspace tokens come from the OAuth install
        // flow (provider_tokens, team_id=<linear org uuid>). OAuth client
        // credentials are needed to refresh expired access tokens.
        {
            let linear = self::linear::adapter::LinearAdapter::new(
                config.linear.webhook_secret.clone(),
                config.linear.oauth_client_id.clone(),
                config.linear.oauth_client_secret.clone(),
                db,
            );
            providers.register_linear(Arc::new(linear));
            tracing::info!("Linear adapter configured");
        }

        Some(providers)
    }

    /// Insert into the trait-object map only.
    ///
    /// Private: every adapter kind that has a typed handle must go through its
    /// typed registrar, or the map and the handle could disagree. Widen this to
    /// `pub(crate)` if a stand-in registry is ever needed from another module.
    fn register_trait_object(&mut self, provider: Arc<dyn Adapter>) {
        self.providers.insert(provider.adapter_kind(), provider);
    }

    /// The Webhook adapter has no concrete-only methods, so it has no typed
    /// handle and lives in the map alone.
    pub fn register_webhook(&mut self, provider: Arc<self::webhook::adapter::WebhookAdapter>) {
        self.register_trait_object(provider);
    }

    pub fn register_github_pr(&mut self, provider: Arc<self::github_pr::adapter::GithubPrAdapter>) {
        self.register_trait_object(provider.clone());
        self.github_pr = Some(provider);
    }

    pub fn register_linear(&mut self, provider: Arc<self::linear::adapter::LinearAdapter>) {
        self.register_trait_object(provider.clone());
        self.linear = Some(provider);
    }

    pub fn register_slack(&mut self, provider: Arc<self::slack::adapter::SlackAdapter>) {
        self.register_trait_object(provider.clone());
        self.slack = Some(provider);
    }

    pub fn register_telegram(&mut self, provider: Arc<self::telegram::adapter::TelegramAdapter>) {
        self.register_trait_object(provider.clone());
        self.telegram = Some(provider);
    }

    /// Typed handle to the GitHub PR adapter, for code inside its own module.
    pub fn github_pr(&self) -> Option<&self::github_pr::adapter::GithubPrAdapter> {
        self.github_pr.as_deref()
    }

    /// Typed handle to the Linear adapter, for code inside its own module.
    pub fn linear(&self) -> Option<&self::linear::adapter::LinearAdapter> {
        self.linear.as_deref()
    }

    /// Typed handle to the Slack adapter, for code inside its own module.
    pub fn slack(&self) -> Option<&self::slack::adapter::SlackAdapter> {
        self.slack.as_deref()
    }

    /// Typed handle to the Telegram adapter, for code inside its own module.
    pub fn telegram(&self) -> Option<&self::telegram::adapter::TelegramAdapter> {
        self.telegram.as_deref()
    }

    pub fn get(&self, adapter_kind: AdapterKind) -> Option<&Arc<dyn Adapter>> {
        self.providers.get(&adapter_kind)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&Arc<dyn Adapter>> {
        if let Some(provider) = AdapterKind::parse(name).and_then(|ct| self.providers.get(&ct)) {
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

    /// Returns configured adapter kinds.
    pub fn registered_types(&self) -> Vec<AdapterKind> {
        self.providers.keys().copied().collect()
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

    let api = self::slack::api::SlackApi::new(bot_token);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::models::OutboundReply;
    use crate::db::DbPool;

    struct DummyProvider;

    #[async_trait::async_trait]
    impl Adapter for DummyProvider {
        fn adapter_kind(&self) -> AdapterKind {
            AdapterKind::Telegram
        }

        async fn resolve_user(
            &self,
            _identity: &str,
            _db: &DbPool,
        ) -> Result<Option<String>, AdapterError> {
            Ok(None)
        }

        async fn send_reply(
            &self,
            _thread: &crate::adapters::models::BindingContext,
            _config: &crate::adapters::models::LoopContext,
            _message: &OutboundReply,
        ) -> Result<Option<String>, AdapterError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn default_post_to_destination_returns_bad_request() {
        let provider = DummyProvider;
        let reply = OutboundReply {
            text: "hi".into(),
            task_url: None,
            footer: None,
            author_user_id: None,
            is_error: false,
            files: vec![],
        };
        let err = provider
            .post_to_destination("chat-123", &reply)
            .await
            .expect_err("default impl should error");
        match err {
            AdapterError::BadRequest(msg) => {
                assert!(
                    msg.contains("does not support direct destination sends"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected BadRequest, got: {other:?}"),
        }
    }

    /// The user-DM handler turns `None` into the "not supported" rejection, so
    /// an adapter with no DM surface keeps rejecting by inheriting this default.
    #[tokio::test]
    async fn default_dm_destination_is_none() {
        let provider = DummyProvider;
        assert_eq!(
            provider
                .dm_destination("U123")
                .await
                .expect("default impl should not error"),
            None
        );
    }

    #[test]
    fn empty_registry_takes_a_stand_in_adapter() {
        let mut providers = Adapters::new();
        providers.register_trait_object(Arc::new(DummyProvider));

        assert_eq!(providers.registered_types(), vec![AdapterKind::Telegram]);
        assert!(providers.get(AdapterKind::Telegram).is_some());
    }
}
