//! Adapter-layer router for events delivered to an already-bound external subject.
//!
//! Adapter runtimes still decide how to parse a subject and which bindings
//! exist. The shared binding runtime owns recipient resolution, receipt
//! deduplication, self-echo filtering, and sandbox notification.

use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, InboundMessage, SubscriptionMatch, TaskBinding};
use crate::state::AppState;

/// One Adapter event and the active bindings listening to its subject.
pub(crate) struct SubjectEvent<'a> {
    /// Adapter label used for message rows and metrics.
    pub(crate) adapter_label: &'a str,
    /// Adapter key used for subject bindings and event receipts.
    pub(crate) binding_provider: &'a str,
    pub(crate) subject_kind: &'a str,
    pub(crate) subject_key: &'a str,
    pub(crate) bindings: Vec<TaskBinding>,
    /// Task provenance names as the author of the event.
    pub(crate) author_task_id: Option<&'a str>,
    /// Binding created by this event, if any.
    pub(crate) new_task_binding_id: Option<&'a str>,
}

/// Route an Adapter event to the tasks bound to its external subject.
pub(crate) async fn route_subject_event(
    state: &AppState,
    connection: &AdapterConnection,
    subscription: &SubscriptionMatch,
    msg: &InboundMessage,
    event: SubjectEvent<'_>,
) -> Result<String, AdapterError> {
    crate::bindings::runtime::fan_out(
        state,
        connection,
        subscription,
        msg,
        crate::bindings::runtime::Fanout {
            adapter_label: event.adapter_label,
            binding_provider: event.binding_provider,
            subject_kind: event.subject_kind,
            subject_key: event.subject_key,
            bindings: event.bindings,
            author_task_id: event.author_task_id,
            new_task_binding_id: event.new_task_binding_id,
        },
    )
    .await
}
