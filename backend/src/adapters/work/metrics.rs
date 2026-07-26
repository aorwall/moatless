//! Prometheus series for durable adapter work.
//!
//! Cardinality: provider × kind × outcome, where both provider and kind are
//! compile-time constants supplied at the admission call site — a bounded
//! vocabulary, matching the note in [`crate::adapters::metrics`].

use super::store::Backlog;

pub(crate) const ADAPTER_WORK_ADMITTED_TOTAL: &str = "moatless_adapter_work_admitted_total";
pub(crate) const ADAPTER_WORK_EXECUTED_TOTAL: &str = "moatless_adapter_work_executed_total";
pub(crate) const ADAPTER_WORK_BACKLOG: &str = "moatless_adapter_work_backlog";
pub(crate) const ADAPTER_WORK_OLDEST_PENDING_SECONDS: &str =
    "moatless_adapter_work_oldest_pending_seconds";

/// Register the adapter work metric descriptions with the Prometheus recorder.
pub fn describe_adapter_work_metrics() {
    metrics::describe_counter!(
        ADAPTER_WORK_ADMITTED_TOTAL,
        "Durable adapter work admission outcomes (admitted | duplicate | at_capacity | error) by provider and kind"
    );
    metrics::describe_counter!(
        ADAPTER_WORK_EXECUTED_TOTAL,
        "Durable adapter work execution outcomes (succeeded | retry | terminal) by provider and kind"
    );
    metrics::describe_gauge!(
        ADAPTER_WORK_BACKLOG,
        "Durable adapter work records by status (pending | claimed | failed)"
    );
    metrics::describe_gauge!(
        ADAPTER_WORK_OLDEST_PENDING_SECONDS,
        "Age of the oldest adapter work record still waiting to be executed"
    );
}

/// Record one admission outcome. `outcome` is one of `admitted`, `duplicate`,
/// `at_capacity`, `error`.
pub(crate) fn record_admission(provider: &'static str, kind: &'static str, outcome: &'static str) {
    metrics::counter!(
        ADAPTER_WORK_ADMITTED_TOTAL,
        "provider" => provider,
        "kind" => kind,
        "outcome" => outcome,
    )
    .increment(1);
}

/// Record one execution outcome. `outcome` is one of `succeeded`, `retry`,
/// `terminal`.
///
/// `terminal` is the series to alert on: it counts webhooks the backend
/// accepted and then gave up on.
pub(crate) fn record_execution(provider: &str, kind: &str, outcome: &'static str) {
    metrics::counter!(
        ADAPTER_WORK_EXECUTED_TOTAL,
        "provider" => provider.to_string(),
        "kind" => kind.to_string(),
        "outcome" => outcome,
    )
    .increment(1);
}

/// Publish the backlog gauges.
///
/// `oldest_pending_seconds` is the signal that says admitted work has stopped
/// moving — the readiness flip alone cannot say that, because a replica whose
/// worker died still reports the backlog another replica is draining.
pub(crate) fn record_backlog(backlog: &Backlog) {
    metrics::gauge!(ADAPTER_WORK_BACKLOG, "status" => "pending").set(backlog.pending as f64);
    metrics::gauge!(ADAPTER_WORK_BACKLOG, "status" => "claimed").set(backlog.claimed as f64);
    metrics::gauge!(ADAPTER_WORK_BACKLOG, "status" => "failed").set(backlog.failed as f64);
    metrics::gauge!(ADAPTER_WORK_OLDEST_PENDING_SECONDS).set(backlog.oldest_pending_secs as f64);
}
