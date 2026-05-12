//! Prometheus counters for trigger execution.
//!
//! `trigger_id` is emitted as a label so the admin UI can render per-trigger
//! counts. Global tick / leader-election failures pass `""` because they
//! aren't tied to a specific trigger.

pub(crate) const TRIGGER_JOB_TOTAL: &str = "moatless_trigger_job_total";

/// Register trigger metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_trigger_metrics() {
    metrics::describe_counter!(
        TRIGGER_JOB_TOTAL,
        "Scheduled trigger job outcomes (success | ignored | error) by trigger and reason"
    );
}

/// Record one scheduled-job outcome.
///
/// `outcome` must be one of `"success"`, `"ignored"`, `"error"`.
/// `reason` is a fixed-vocabulary label (e.g. `"not_leader"`, `"no_lock"`,
/// `"execution_failed"`); pass `""` when not applicable.
/// `trigger_id` is the job's trigger ID, or `""` for global failures.
pub(crate) fn record_job(outcome: &'static str, reason: &'static str, trigger_id: &str) {
    metrics::counter!(
        TRIGGER_JOB_TOTAL,
        "outcome" => outcome,
        "reason" => reason,
        "trigger_id" => trigger_id.to_string(),
    )
    .increment(1);
}
