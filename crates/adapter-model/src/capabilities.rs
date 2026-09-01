//! Adapter capability interfaces and shared capability value types.
//!
//! What an Adapter *offers* — reply to a binding, reply to one message, post to
//! a raw destination, react, enrich inbound context, report external status.
//! Each is a separate trait so a provider implements only what it can do and
//! the registry can ask for one capability without naming a concrete adapter.
//!
//! These traits are the Adapter contract, so they live in the kernel with the
//! vocabulary they speak and the error they return. Nothing here names a store,
//! a runtime, or a band above this crate: a provider crate implements them
//! without depending on whatever is orchestrating it.

use crate::error::{AdapterError, SharedAdapterError};
use crate::binding::binding_context;
use crate::loops::subscription_context;
use moatless_types::adapter::{
    AdapterConnection, AdapterKind, BindingContext, BindingDestination, DestinationPostResult,
    ExternalStatus, InboundMessage, OutboundReply, TaskBinding,
};
use moatless_types::loops::{LoopContext, SubscriptionMatch};

/// Info returned by `send_message` to create a new `BindingContext`.
#[derive(Debug)]
pub struct NewThreadInfo {
    /// Adapter-specific thread/message ID (e.g., Slack message `ts`).
    pub external_thread_id: String,
    /// Adapter-specific properties for the thread (e.g., `channel`, `thread_ts`).
    pub properties: serde_json::Value,
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

/// Build persisted binding destinations from Adapter thread data.
pub trait AdapterDestinationBuilder: Adapter {
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
}

/// Shared message formatting limits for outbound capabilities that send text.
pub trait AdapterOutboundContent: Adapter {
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
pub trait AdapterBindingReply: AdapterOutboundContent {
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
            return Err(SharedAdapterError::BadRequest(
                "Provider needs a trigger-backed binding for outbound replies".into(),
            )
            .into());
        };
        let config = subscription_context(rule);
        let thread = binding_context(binding);
        self.send_reply(&thread, &config, message).await
    }
}

/// Reply to one specific external message.
#[async_trait::async_trait]
pub trait AdapterMessageReply: AdapterOutboundContent {
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
pub trait AdapterDirectDestination: AdapterOutboundContent {
    /// Post to an Adapter-specific destination by raw ID.
    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<DestinationPostResult, AdapterError>;
}

/// Resolve direct-message destinations for provider-side users.
#[async_trait::async_trait]
pub trait AdapterDirectMessage: Adapter {
    /// Resolve the destination that addresses a single external user directly.
    async fn dm_destination(&self, provider_user_id: &str) -> Result<Option<String>, AdapterError>;
}

/// Optional follow-up after a direct destination send creates a binding.
#[async_trait::async_trait]
pub trait AdapterPostBindingFollowup: Adapter {
    /// Follow-up the provider runs after an outbound send created a binding.
    ///
    /// The implementor owns the ports this needs (a task store, its frontend
    /// URL) rather than receiving `AppState`, so the capability surface names
    /// nothing in the server band.
    async fn on_binding_created(
        &self,
        connection: &AdapterConnection,
        binding: &TaskBinding,
        task_id: &str,
    );
}

/// Reaction support for external messages.
#[async_trait::async_trait]
pub trait AdapterReactions: Adapter {
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
pub trait AdapterInboundPresentation: Send + Sync {
    fn external_id_aliases(&self, external_id: &str) -> Vec<String> {
        vec![external_id.to_string()]
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
pub trait AdapterBindingStatus: Send + Sync {
    fn binding_status_key(&self, _binding: &TaskBinding) -> Option<String> {
        None
    }

    fn binding_label_and_url(&self, binding: &TaskBinding) -> (String, Option<String>) {
        (binding.subject_key.clone(), None)
    }

    fn is_terminal_state_kind(&self, _state_kind: &str) -> bool {
        false
    }

    /// Whether the binding's persisted status blob carries every field this
    /// adapter now writes. An adapter whose blob shape grew answers `false` for
    /// a binding written before the growth, so a terminal binding is refreshed
    /// once to fill the missing fields; the default is that nothing is missing.
    fn persisted_status_is_complete(&self, _binding: &TaskBinding) -> bool {
        true
    }

    async fn fetch_binding_status(
        &self,
        _binding: &TaskBinding,
        _acting_user: &str,
    ) -> Option<ExternalStatus> {
        None
    }
}
