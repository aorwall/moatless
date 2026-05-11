//! Prometheus counters for binding inbound routing and outbound send.

pub(crate) const BINDING_INBOUND_TOTAL: &str = "moatless_binding_inbound_total";
pub(crate) const BINDING_OUTBOUND_TOTAL: &str = "moatless_binding_outbound_total";

/// Register binding metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_binding_metrics() {
    metrics::describe_counter!(
        BINDING_INBOUND_TOTAL,
        "Inbound binding routing outcomes (success | ignored | error) by provider and reason"
    );
    metrics::describe_counter!(
        BINDING_OUTBOUND_TOTAL,
        "Outbound binding send outcomes (success | ignored | error) by provider and reason"
    );
}

/// Record one inbound routing outcome.
pub(crate) fn record_inbound(provider: &str, outcome: &'static str, reason: &'static str) {
    metrics::counter!(
        BINDING_INBOUND_TOTAL,
        "provider" => provider.to_string(),
        "outcome" => outcome,
        "reason" => reason,
    )
    .increment(1);
}

/// Record one outbound send outcome.
pub(crate) fn record_outbound(provider: &str, outcome: &'static str, reason: &'static str) {
    metrics::counter!(
        BINDING_OUTBOUND_TOTAL,
        "provider" => provider.to_string(),
        "outcome" => outcome,
        "reason" => reason,
    )
    .increment(1);
}
