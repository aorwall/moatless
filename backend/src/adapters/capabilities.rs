//! Adapter capability interfaces and shared capability value types.

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, ExternalStatus, InboundMessage, LoopContext, NewThreadInfo,
    OutboundReply, SubscriptionMatch, TaskBinding,
};
use crate::db::DbPool;
use crate::state::AppState;

use moatless_api_schemas::enums::AdapterKind;

/// Result of posting to a provider-specific destination by raw ID.
///
/// Returned by [`AdapterDirectDestination::post_to_destination`] and used by
/// callers to build a matching [`TaskBinding`] row when the post is associated
/// with a task.
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

/// Universal adapter identity kept for compatibility with typed provider
/// registration and legacy name lookups.
pub trait Adapter: Send + Sync {
    /// Adapter type identifier.
    fn adapter_kind(&self) -> AdapterKind;

    /// Canonical Adapter key for external messages and external labels.
    fn provider_key(&self) -> &'static str {
        self.adapter_kind().as_str()
    }

    /// Adapter key used by Adapter connections and Subscription sources.
    ///
    /// Most adapters store the same key everywhere. GitHub PR uses the
    /// user-facing Adapter key (`github`) for Adapter connections and Subscriptions while
    /// task bindings and message metadata use the adapter target key (`github_pr`).
    fn connection_provider_key(&self) -> &'static str {
        self.provider_key()
    }
}

/// Universal identity surface every registered Adapter exposes.
pub(crate) trait AdapterIdentity: Send + Sync {
    /// Adapter type identifier.
    fn adapter_kind(&self) -> AdapterKind;

    /// Canonical Adapter key for external messages and external labels.
    fn provider_key(&self) -> &'static str {
        self.adapter_kind().as_str()
    }

    /// Adapter key used by Adapter connections and Subscription sources.
    fn connection_provider_key(&self) -> &'static str {
        self.provider_key()
    }
}

impl<T: Adapter + ?Sized> AdapterIdentity for T {
    fn adapter_kind(&self) -> AdapterKind {
        Adapter::adapter_kind(self)
    }

    fn provider_key(&self) -> &'static str {
        Adapter::provider_key(self)
    }

    fn connection_provider_key(&self) -> &'static str {
        Adapter::connection_provider_key(self)
    }
}

/// Resolve Adapter-side identities into Moatless user identities.
#[async_trait::async_trait]
pub(crate) trait AdapterUserIdentity: Send + Sync {
    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, AdapterError>;
}

/// Build persisted binding destinations from Adapter thread data.
pub(crate) trait AdapterDestinationBuilder: AdapterIdentity {
    /// Scope value used to find the Adapter connection for a thread binding.
    fn connection_scope_value_from_existing_thread(
        &self,
        _source_id: &str,
        _thread_properties: &serde_json::Value,
    ) -> Result<String, AdapterError> {
        Ok("default".to_string())
    }

    /// Whether Adapter connection lookup should fall back to the `"default"` scope.
    fn allow_default_connection_fallback(&self) -> bool {
        true
    }

    /// Source matcher used to find the Subscription for a thread binding.
    fn trigger_source_matcher_from_existing_thread(
        &self,
        source_id: &str,
        _thread_properties: &serde_json::Value,
    ) -> Result<String, AdapterError> {
        Ok(source_id.to_string())
    }

    /// Whether Subscription lookup should fall back to a wildcard source matcher.
    fn allow_wildcard_trigger_fallback(&self) -> bool {
        true
    }

    /// Build the binding destination for an existing external thread.
    fn binding_destination_from_existing_thread(
        &self,
        source_id: &str,
        external_thread_id: &str,
        thread_properties: &serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, AdapterError>;

    /// Build the binding destination for a newly-created external thread.
    ///
    /// Compatibility bridge for destination builders that can bind existing
    /// subjects but do not create new threads. Callers only reach this after
    /// obtaining an [`AdapterNewThread`] capability, so unsupported new-thread
    /// workflows still fail at capability lookup.
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
}

/// Shared message formatting limits for outbound capabilities that send text.
pub(crate) trait AdapterOutboundContent: AdapterIdentity {
    /// Whether this Adapter supports file attachments in outbound messages.
    fn supports_files(&self) -> bool {
        false
    }

    /// Maximum length (in characters) for an outbound reply's text body.
    fn max_message_length(&self) -> usize {
        4000
    }
}

/// Send replies to an existing task binding.
#[async_trait::async_trait]
pub(crate) trait AdapterBindingReply: AdapterOutboundContent {
    /// Send a reply back to the adapter target thread.
    async fn send_reply(
        &self,
        thread: &BindingContext,
        config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError>;

    /// Send a reply back to the subject identified by a `TaskBinding`.
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
}

/// Reply to one specific external message.
#[async_trait::async_trait]
pub(crate) trait AdapterMessageReply: AdapterOutboundContent {
    /// Reply specifically to the message identified by `target_external_id`.
    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError>;
}

/// Post to an Adapter-specific destination by raw ID.
#[async_trait::async_trait]
pub(crate) trait AdapterDirectDestination: AdapterOutboundContent {
    /// Post to an Adapter-specific destination by raw ID.
    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<DestinationPostResult, AdapterError>;
}

/// Resolve direct-message destinations for provider-side users.
#[async_trait::async_trait]
pub(crate) trait AdapterDirectMessage: AdapterIdentity {
    /// Resolve the destination that addresses a single external user directly.
    async fn dm_destination(&self, provider_user_id: &str) -> Result<Option<String>, AdapterError>;
}

/// Optional follow-up after a direct destination send creates a binding.
#[async_trait::async_trait]
pub(crate) trait AdapterPostBindingFollowup: AdapterIdentity {
    /// Follow-up the provider runs after an outbound send created a binding.
    async fn on_binding_created(
        &self,
        state: &AppState,
        connection: &AdapterConnection,
        binding: &TaskBinding,
        task_id: &str,
    );
}

/// Send new top-level external threads.
#[async_trait::async_trait]
pub(crate) trait AdapterNewThread: AdapterOutboundContent {
    /// Send a new top-level message to an external destination.
    async fn send_message(
        &self,
        config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<NewThreadInfo, AdapterError>;
}

/// Reaction support for external messages.
#[async_trait::async_trait]
pub(crate) trait AdapterReactions: AdapterIdentity {
    async fn add_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError>;

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<(), AdapterError>;
}

/// Adapter-specific inbound formatting and context enrichment.
#[async_trait::async_trait]
pub(crate) trait AdapterInboundPresentation: Send + Sync {
    fn external_id_aliases(&self, external_id: &str) -> Vec<String> {
        vec![external_id.to_string()]
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "presentation capability keeps the sender-label hook for adapter-specific UI polish"
        )
    )]
    #[cfg_attr(
        test,
        allow(
            dead_code,
            reason = "test target may not exercise every optional presentation hook"
        )
    )]
    fn sender_label_suffix(&self, _msg: &InboundMessage) -> Option<String> {
        None
    }

    fn inbound_message_prefix(&self, _msg: &InboundMessage) -> Option<String> {
        None
    }

    fn reply_instructions(&self) -> Option<String> {
        None
    }

    fn thread_reply_hint(&self, _msg: &InboundMessage) -> Option<String> {
        None
    }

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

/// External status metadata and live status fetching for a binding subject.
#[async_trait::async_trait]
pub(crate) trait AdapterBindingStatus: Send + Sync {
    fn binding_status_key(&self, _binding: &TaskBinding) -> Option<String> {
        None
    }

    fn binding_label_and_url(&self, binding: &TaskBinding) -> (String, Option<String>) {
        (binding.subject_key.clone(), None)
    }

    fn is_terminal_state_kind(&self, _state_kind: &str) -> bool {
        false
    }

    async fn fetch_binding_status(
        &self,
        _binding: &TaskBinding,
        _acting_user: &str,
    ) -> Option<ExternalStatus> {
        None
    }
}
