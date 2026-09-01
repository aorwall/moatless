//! The Binding domain the Adapter kernel publishes: the read seam, the origin
//! taxonomy, and the vocabulary a provider builds from an inbound event.
//!
//! An Adapter that only *asks about* Bindings reads [`BindingProbe`]. It is
//! declared in the Adapter kernel, which every consumer already names for the
//! Adapter contract; the Binding store implements it in the `moatless-loops`
//! crate, beside the table it reads. An Adapter that *drives* the Binding
//! lifecycle names that store directly — a narrow read port beside a store a
//! crate already holds is a second surface, not a seam.
//!
//! The origin constants are the declared `task_bindings.origin` values adapter
//! routing uses; the column is an open string and other writers use values
//! outside this set. [`binding_context`] projects a stored Binding into the
//! shape a provider replies against.

use async_trait::async_trait;

use moatless_types::adapter::{BindingContext, TaskBinding};
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

/// The declared `task_bindings.origin` values adapter routing uses, and the
/// role taxonomy: the direction in which the task and the external subject met.
///
/// [`ADAPTER_EVENT_BINDING_ORIGIN`] is the responder direction — the subject
/// existed first and a loop spawned a task from it, so it is the one origin the
/// router writes from an inbound event. Every other origin is a producer: the
/// task existed first and reached out.
///
/// The string values are fixed; existing database rows and the API depend on
/// them.
pub const ADAPTER_EVENT_BINDING_ORIGIN: &str = "ingress";
/// Producer: the task replied into a subject, or was connected to one.
pub const CONNECTED_THREAD_BINDING_ORIGIN: &str = "connected_thread";
/// Producer: a CLI-driven endpoint filed the subject on the task's behalf.
pub const TASK_OWNED_BINDING_ORIGIN: &str = "task_owned";

/// Build a provider binding context from a task binding. Never persist this row.
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
