//! Prometheus counters for Loop firings.
//!
//! `loop_id` is emitted as a label so the admin UI can render per-Loop counts.
//! Global tick / leader-election failures pass `""` because they aren't tied to
//! a specific Loop.

pub(crate) const LOOP_SCHEDULE_FIRING_TOTAL: &str = "moatless_loop_schedule_firing_total";
pub(crate) const LOOP_TASK_REFUSED_TOTAL: &str = "moatless_loop_task_refused_total";

/// Register Loop firing metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_loop_firing_metrics() {
    metrics::describe_counter!(
        LOOP_SCHEDULE_FIRING_TOTAL,
        "Loop schedule firing outcomes (success | ignored | error) by Loop and reason"
    );
    metrics::describe_counter!(
        LOOP_TASK_REFUSED_TOTAL,
        "Loop firings whose Task preflight refused, so no agent ran, by Loop and adapter"
    );
}

/// Record one schedule-firing outcome.
///
/// `outcome` is `"success"`, `"ignored"`, or `"error"`; `reason` is a fixed
/// label (e.g. `"not_leader"`) or `""`; `loop_id` is `""` for global failures.
pub(crate) fn record_schedule_firing(outcome: &'static str, reason: &'static str, loop_id: &str) {
    metrics::counter!(
        LOOP_SCHEDULE_FIRING_TOTAL,
        "outcome" => outcome,
        "reason" => reason,
        "loop_id" => loop_id.to_string(),
    )
    .increment(1);
}

/// Record one Loop firing whose Task was refused at preflight.
///
/// The firing paths only log a refusal and the Task sits errored with nobody
/// told, so this counter is what an alert watches.
pub(crate) fn record_task_refused(loop_id: &str, adapter: &str) {
    metrics::counter!(
        LOOP_TASK_REFUSED_TOTAL,
        "loop_id" => loop_id.to_string(),
        "adapter" => adapter.to_string(),
    )
    .increment(1);
}
