//! Prometheus series for durable Adapter delivery.
//!
//! Cardinality: sender × kind × outcome, where both sender and kind are
//! compile-time constants supplied at the admission call site — a bounded
//! vocabulary, matching the note in [`moatless_adapter_model::metrics`].

use super::dao::Backlog;

pub(crate) const ADAPTER_DELIVERY_ADMITTED_TOTAL: &str = "moatless_adapter_delivery_admitted_total";
pub(crate) const ADAPTER_DELIVERY_EXECUTED_TOTAL: &str = "moatless_adapter_delivery_executed_total";
pub(crate) const ADAPTER_DELIVERY_BACKLOG: &str = "moatless_adapter_delivery_backlog";
pub(crate) const ADAPTER_DELIVERY_OLDEST_PENDING_SECONDS: &str =
    "moatless_adapter_delivery_oldest_pending_seconds";

/// Register the Adapter delivery metric descriptions with the Prometheus
/// recorder.
pub fn describe_adapter_delivery_metrics() {
    metrics::describe_counter!(
        ADAPTER_DELIVERY_ADMITTED_TOTAL,
        "Durable Adapter delivery admission outcomes (admitted | duplicate | at_capacity | error) by adapter and kind"
    );
    metrics::describe_counter!(
        ADAPTER_DELIVERY_EXECUTED_TOTAL,
        "Durable Adapter delivery execution outcomes (succeeded | retry | throttled | terminal) by adapter and kind"
    );
    metrics::describe_gauge!(
        ADAPTER_DELIVERY_BACKLOG,
        "Durable Adapter delivery records by status (pending | claimed | failed)"
    );
    metrics::describe_gauge!(
        ADAPTER_DELIVERY_OLDEST_PENDING_SECONDS,
        "Age of the oldest Adapter delivery record still waiting to be executed"
    );
}

/// Record one admission outcome. `outcome` is one of `admitted`, `duplicate`,
/// `at_capacity`, `error`.
pub(crate) fn record_admission(adapter: &'static str, kind: &'static str, outcome: &'static str) {
    metrics::counter!(
        ADAPTER_DELIVERY_ADMITTED_TOTAL,
        "adapter" => adapter,
        "kind" => kind,
        "outcome" => outcome,
    )
    .increment(1);
}

/// Record one execution outcome. `outcome` is one of `succeeded`, `retry`,
/// `throttled`, `terminal`.
///
/// `terminal` is the series to alert on: it counts webhooks the backend
/// accepted and then gave up on.
pub(crate) fn record_execution(adapter: &str, kind: &str, outcome: &'static str) {
    metrics::counter!(
        ADAPTER_DELIVERY_EXECUTED_TOTAL,
        "adapter" => adapter.to_string(),
        "kind" => kind.to_string(),
        "outcome" => outcome,
    )
    .increment(1);
}

/// Publish the backlog gauges.
///
/// `oldest_pending_seconds` is the signal that admitted deliveries stopped
/// moving; the readiness flip alone can't say that, since a replica whose
/// worker died still reports the backlog another replica is draining.
pub(crate) fn record_backlog(backlog: &Backlog) {
    metrics::gauge!(ADAPTER_DELIVERY_BACKLOG, "status" => "pending").set(backlog.pending as f64);
    metrics::gauge!(ADAPTER_DELIVERY_BACKLOG, "status" => "claimed").set(backlog.claimed as f64);
    metrics::gauge!(ADAPTER_DELIVERY_BACKLOG, "status" => "failed").set(backlog.failed as f64);
    metrics::gauge!(ADAPTER_DELIVERY_OLDEST_PENDING_SECONDS)
        .set(backlog.oldest_pending_secs as f64);
}
