//! Prometheus counters for adapter webhook ingress.
//!
//! `trigger_id` and `connection_id` are emitted as labels so the admin UI
//! can render per-trigger and per-connection counts. Pass empty strings
//! at call sites where the IDs aren't yet known (e.g. before rule lookup).
//!
//! Cardinality note: number of active triggers × connections × adapters ×
//! outcomes × reasons. Keep the `reason` vocabulary small and prune unused
//! triggers/connections to avoid runaway series in Prometheus.

pub(crate) const ADAPTER_WEBHOOK_TOTAL: &str = "moatless_adapter_webhook_total";

/// Register adapter metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_adapter_metrics() {
    metrics::describe_counter!(
        ADAPTER_WEBHOOK_TOTAL,
        "Adapter webhook outcomes (success | ignored | error) by adapter, trigger, connection, and reason"
    );
}

/// Record one adapter webhook outcome.
///
/// `outcome` must be one of `"success"`, `"ignored"`, `"error"`.
/// `reason` is a fixed-vocabulary label (e.g. `"inactive"`, `"no_trigger"`,
/// `"delivery_failed"`); pass `""` when not applicable.
/// `trigger_id` / `connection_id` are passed as empty strings when not
/// resolved yet (e.g. early validation failures).
pub(crate) fn record_webhook(
    adapter: &'static str,
    outcome: &'static str,
    reason: &'static str,
    trigger_id: &str,
    connection_id: &str,
) {
    metrics::counter!(
        ADAPTER_WEBHOOK_TOTAL,
        "adapter" => adapter,
        "outcome" => outcome,
        "reason" => reason,
        "trigger_id" => trigger_id.to_string(),
        "connection_id" => connection_id.to_string(),
    )
    .increment(1);
}
