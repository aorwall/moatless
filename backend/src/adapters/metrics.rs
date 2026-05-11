//! Prometheus counters for adapter webhook ingress.

pub(crate) const ADAPTER_WEBHOOK_TOTAL: &str = "moatless_adapter_webhook_total";

/// Register adapter metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_adapter_metrics() {
    metrics::describe_counter!(
        ADAPTER_WEBHOOK_TOTAL,
        "Adapter webhook outcomes (success | ignored | error) by adapter and reason"
    );
}

/// Record one adapter webhook outcome.
///
/// `outcome` must be one of `"success"`, `"ignored"`, `"error"`.
/// `reason` is a fixed-vocabulary label (e.g. `"inactive"`, `"no_trigger"`,
/// `"delivery_failed"`); pass `""` when not applicable.
pub(crate) fn record_webhook(adapter: &'static str, outcome: &'static str, reason: &'static str) {
    metrics::counter!(
        ADAPTER_WEBHOOK_TOTAL,
        "adapter" => adapter,
        "outcome" => outcome,
        "reason" => reason,
    )
    .increment(1);
}
