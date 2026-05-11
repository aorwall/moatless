//! Prometheus counters for trigger execution.

pub(crate) const TRIGGER_JOB_TOTAL: &str = "moatless_trigger_job_total";

/// Register trigger metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_trigger_metrics() {
    metrics::describe_counter!(
        TRIGGER_JOB_TOTAL,
        "Scheduled trigger job outcomes (success | ignored | error) by reason"
    );
}

/// Record one scheduled-job outcome.
///
/// `outcome` must be one of `"success"`, `"ignored"`, `"error"`.
/// `reason` is a fixed-vocabulary label (e.g. `"not_leader"`, `"no_lock"`,
/// `"execution_failed"`); pass `""` when not applicable.
pub(crate) fn record_job(outcome: &'static str, reason: &'static str) {
    metrics::counter!(
        TRIGGER_JOB_TOTAL,
        "outcome" => outcome,
        "reason" => reason,
    )
    .increment(1);
}
