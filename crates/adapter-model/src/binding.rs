//! The Binding domain the Adapter kernel publishes: the read seam, the origin
//! taxonomy, and the vocabulary a provider builds from an inbound event.
//!
//! An Adapter that only *asks about* Bindings reads [`BindingProbe`],
//! implemented by the Binding store in `moatless-loops` beside its table; one
//! that *drives* the Binding lifecycle names that store directly.
//!
//! Origin constants are the declared `task_bindings.origin` values adapter
//! routing uses — the column is an open string, and other writers use values
//! outside this set.

use async_trait::async_trait;

use chrono::{DateTime, Utc};
use moatless_types::adapter::TaskBinding;
use moatless_types::store::StoreError;

/// Whether a subject already has a Binding. Answers a yes/no; the rows behind
/// it stay in the crate that owns the table.
#[async_trait]
pub trait BindingProbe: Send + Sync {
    /// Whether any Binding tracks this subject.
    async fn is_bound(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<bool, StoreError>;
}

/// The `task_bindings.origin` role taxonomy: the direction the task and
/// external subject met.
///
/// This is the responder direction — the router writes it from an inbound
/// event; every other origin is a producer where the task existed first. Values
/// are fixed: existing rows and the API depend on them.
pub const ADAPTER_EVENT_BINDING_ORIGIN: &str = "ingress";
/// Producer: the task replied into a subject, or was connected to one.
pub const CONNECTED_THREAD_BINDING_ORIGIN: &str = "connected_thread";
/// Producer: a CLI-driven endpoint filed the subject on the task's behalf.
pub const TASK_OWNED_BINDING_ORIGIN: &str = "task_owned";

/// Build a provider binding context from a task binding. Never persist this
/// row.
pub fn binding_context(binding: &TaskBinding) -> BindingContext {
    let mut destination_config = binding.destination_config.clone();
    if let Some(obj) = destination_config.as_object_mut() {
        if let Some(ext_id) = binding.last_external_event_id.as_ref() {
            obj.insert("last_external_event_id".into(), serde_json::json!(ext_id));
        }
        if let Some(filter_uid) = binding.metadata.get("filter_user_id").cloned() {
            obj.insert("filter_user_id".into(), filter_uid);
        }
    }

    BindingContext {
        binding_id: binding.id.clone(),
        loop_id: binding.loop_id.clone(),
        task_id: binding.task_id.clone(),
        subject_kind: binding.subject_kind.clone(),
        subject_key: binding.subject_key.clone(),
        destination_config,
        metadata: binding.metadata.clone(),
        last_external_event_id: binding.last_external_event_id.clone(),
        last_message_at: binding.last_event_at,
        created_at: binding.created_at,
    }
}

/// Adapter-facing subject context.
///
/// Synthesized from a task binding before invoking Adapter-specific outbound
/// and context-fetching code.
#[derive(Debug, Clone)]
pub struct BindingContext {
    pub binding_id: String,
    pub loop_id: Option<String>,
    pub task_id: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
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
    /// Optional cursor seeded when the binding is created from an outbound
    /// send.
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
