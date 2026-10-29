//! Adapter capability interfaces and shared capability value types.
//!
//! What an Adapter *offers* — reply to a binding, reply to one message, post to
//! a raw destination, react, enrich inbound context, report external status —
//! each a separate trait, so a provider implements only what it can do and the
//! registry can ask for one capability without naming a concrete adapter.
//!
//! These traits are the Adapter contract: nothing here names a store, a
//! runtime, or a band above this crate, so a provider implements them without
//! depending on whatever orchestrates it.

use crate::binding::binding_context;
use crate::binding::{BindingContext, BindingDestination};
use crate::error::{AdapterError, SharedAdapterError};
use crate::loops::subscription_context;
use crate::outbound::{DestinationPostResult, OutboundReply};
use moatless_types::adapter::{
    AdapterConnection, AdapterKind, ExternalStatus, InboundAttachment, InboundMessage, TaskBinding,
};
use moatless_types::loops::{LoopContext, SubscriptionMatch};

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
    /// Most adapters use the same key everywhere; GitHub PR is the exception —
    /// `github` for connections/Subscriptions, `github_pr` for task bindings
    /// and message metadata.
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

    /// Whether Adapter connection lookup should fall back to the `"default"`
    /// scope.
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

    /// Whether Subscription lookup should fall back to a wildcard source
    /// matcher.
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
    /// React as `acting_user_id`, the already-authorized caller.
    ///
    /// A provider that mints a per-user token needs an identity, and a binding
    /// carries one only when a subscription filter put it there. Passing the
    /// caller lets a binding without one still react.
    async fn add_reaction(
        &self,
        binding: &BindingContext,
        acting_user_id: &str,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError>;

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        acting_user_id: &str,
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

    /// The adapter-specific reply guidance appended to a fired task's system
    /// prompt. Async because an adapter may have to ask the provider who it is
    /// before it can tell the agent — see Slack's `ensure_bot_user_id`.
    async fn reply_instructions(&self) -> Option<String> {
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

/// Fetch the files an inbound event names, for the delivery worker to store.
#[async_trait::async_trait]
pub trait AdapterInboundFiles: Send + Sync {
    /// The bytes of `attachment`, read with `connection`'s credentials.
    ///
    /// Returns an error, never a partial body, when the provider refuses the
    /// read, answers with something other than the file, or sends more than
    /// `max_bytes`, which the read stops at rather than buffering past.
    async fn fetch_attachment(
        &self,
        connection: &AdapterConnection,
        attachment: &InboundAttachment,
        max_bytes: u64,
    ) -> Result<Vec<u8>, AdapterError>;
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
    /// adapter now writes; `false` for a binding written before the blob shape
    /// grew, so a terminal binding is refreshed once to fill the missing
    /// fields.
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

/// One Task Room to open, everything resolved but the Adapter's own calls.
#[derive(Debug, Clone)]
pub struct RoomRequest {
    /// The idempotency key: opening again for the same Task returns its room.
    pub task_id: String,
    pub title: String,
    /// The triggering event's own Binding, when it came through the hosting
    /// Adapter. The room is opened from it.
    pub origin: Option<TaskBinding>,
    /// Where to announce a room with no `origin`, in the hosting Adapter's
    /// destination syntax ([`moatless_types::loops::RoomSpec::anchor`]).
    pub anchor: Option<String>,
    /// Provider account ids to add, already resolved from Moatless users.
    pub invitees: Vec<String>,
    /// See [`moatless_types::loops::RoomSpec::private`].
    pub private: Option<bool>,
    /// `{frontend_url}/tasks/{task_id}`, for the announcement. `None` when no
    /// frontend URL is configured.
    pub task_url: Option<String>,
}

/// A room an Adapter opened: the Binding the Task is tied to it by.
#[derive(Debug, Clone)]
pub struct RoomOpened {
    pub destination: BindingDestination,
}

/// Hosts Task Rooms: a dedicated conversation space per Task, which the Task
/// is bound to for its life.
#[async_trait::async_trait]
pub trait AdapterTaskRoom: Adapter {
    /// Open the room for `request.task_id`, or return the one already open.
    ///
    /// Must be idempotent per `task_id`: the caller is a durable delivery that
    /// can run again after the room was opened. Returns
    /// [`SharedAdapterError::BadRequest`] when `request` has neither an
    /// `origin` nor an `anchor` it can open from.
    async fn open_room(
        &self,
        connection: &AdapterConnection,
        request: &RoomRequest,
    ) -> Result<RoomOpened, AdapterError>;
}
