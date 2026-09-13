//! Prometheus counters for Loop schedule firings.
//!
//! `loop_id` is emitted as a label so the admin UI can render per-Loop counts.
//! Global tick / leader-election failures pass `""` because they aren't tied to
//! a specific Loop.

pub(crate) const LOOP_SCHEDULE_FIRING_TOTAL: &str = "moatless_loop_schedule_firing_total";

/// Register Loop firing metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_loop_firing_metrics() {
    metrics::describe_counter!(
        LOOP_SCHEDULE_FIRING_TOTAL,
        "Loop schedule firing outcomes (success | ignored | error) by Loop and reason"
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
