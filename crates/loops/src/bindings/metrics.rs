//! Prometheus counters for binding outbound send, and the descriptions for both
//! binding counters.
//!
//! `loop_id` and `connection_id` are emitted as labels so the admin UI
//! can render per-loop / per-connection counts.
//!
//! The inbound counter and the `inbound_reason` vocabulary belong to the Adapter
//! kernel (`moatless_adapter_model::metrics`), because a provider crate meters
//! its own ignores. Both descriptions are registered here so one call covers the
//! pair an operator reads together.

use moatless_adapter_model::metrics::BINDING_INBOUND_TOTAL;

pub(crate) const BINDING_OUTBOUND_TOTAL: &str = "moatless_binding_outbound_total";

/// Register binding metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_binding_metrics() {
    metrics::describe_counter!(
        BINDING_INBOUND_TOTAL,
        "Inbound binding routing outcomes (success | ignored | error) by provider, loop, connection, and reason"
    );
    metrics::describe_counter!(
        BINDING_OUTBOUND_TOTAL,
        "Outbound binding send outcomes (success | ignored | error) by provider, loop, connection, and reason"
    );
}

/// Record one outbound send outcome.
pub(crate) fn record_outbound(
    provider: &str,
    outcome: &'static str,
    reason: &'static str,
    loop_id: &str,
    connection_id: &str,
) {
    metrics::counter!(
        BINDING_OUTBOUND_TOTAL,
        "provider" => provider.to_string(),
        "outcome" => outcome,
        "reason" => reason,
        "loop_id" => loop_id.to_string(),
        "connection_id" => connection_id.to_string(),
    )
    .increment(1);
}
