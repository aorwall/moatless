//! Adapter-layer router for events delivered to an already-bound external subject.
//!
//! Adapter runtimes still decide how to parse a subject and which bindings
//! exist. The shared binding runtime owns recipient resolution, receipt
//! deduplication, self-echo filtering, and sandbox notification.

use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, SubscriptionMatch, TaskBinding};
use moatless_types::adapter::InboundMessage;
use crate::bindings::runtime::RoutingService;

/// One Adapter event and the active bindings listening to its subject.
pub struct SubjectEvent<'a> {
    /// Adapter label used for message rows and metrics.
    pub adapter_label: &'a str,
    /// Adapter key used for subject bindings and event receipts.
    pub binding_provider: &'a str,
    pub subject_kind: &'a str,
    pub subject_key: &'a str,
    pub bindings: Vec<TaskBinding>,
    /// Task provenance names as the author of the event.
    pub author_task_id: Option<&'a str>,
    /// Binding created by this event, if any.
    pub new_task_binding_id: Option<&'a str>,
}

/// Route an Adapter event to the tasks bound to its external subject.
pub async fn route_subject_event(
    routing: &RoutingService,
    connection: &AdapterConnection,
    subscription: &SubscriptionMatch,
    msg: &InboundMessage,
    event: SubjectEvent<'_>,
) -> Result<String, AdapterError> {
    routing
        .fan_out(
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
