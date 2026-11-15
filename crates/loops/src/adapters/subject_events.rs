//! Adapter-layer router for events delivered to an already-bound external
//! subject.
//!
//! Adapter runtimes still decide how to parse a subject and which bindings
//! exist. The shared binding runtime owns recipient resolution, receipt
//! deduplication, self-echo filtering, and sandbox notification.

use crate::bindings::runtime::RoutingService;
use moatless_adapter_model::error::AdapterError;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::InboundMessage;
use moatless_types::adapter::TaskBinding;
use moatless_types::loops::SubscriptionMatch;

/// One Adapter event and the active bindings listening to its subject.
pub struct SubjectEvent<'a> {
    /// Adapter label used for message rows and metrics.
    pub adapter_label: &'a str,
    /// Adapter key used for subject bindings and event receipts.
    pub binding_provider: &'a str,
    pub subject_kind: &'a str,
    pub subject_key: &'a str,
    pub bindings: Vec<TaskBinding>,
    /// The Task that made the event; see [`RoutingService::made_by`].
    pub made_by: Option<&'a str>,
    /// Binding created by this event, if any.
    pub new_task_binding_id: Option<&'a str>,
}

/// Route an Adapter event to the tasks bound to its external subject; the task
/// it reached, as `RoutingService::fan_out` answers it.
pub async fn route_subject_event(
    routing: &RoutingService,
    connection: &AdapterConnection,
    subscription: &SubscriptionMatch,
    msg: &InboundMessage,
    event: SubjectEvent<'_>,
) -> Result<Option<String>, AdapterError> {
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
                made_by: event.made_by,
                new_task_binding_id: event.new_task_binding_id,
            },
        )
        .await
}
