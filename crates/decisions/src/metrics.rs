//! Prometheus metrics for decisions.

use std::time::Duration;

use moatless_api_schemas::decisions::{DecisionBand, DecisionUsage};

const REQUESTS_TOTAL: &str = "moatless_decision_requests_total";
const DURATION_SECONDS: &str = "moatless_decision_duration_seconds";
const BAND_TOTAL: &str = "moatless_decision_band_total";
const INPUT_TOKENS_TOTAL: &str = "moatless_decision_input_tokens_total";
const OUTPUT_TOKENS_TOTAL: &str = "moatless_decision_output_tokens_total";
const PROVIDER_RETRIES_TOTAL: &str = "moatless_decision_provider_retries_total";

/// Register decision metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_decision_metrics() {
    metrics::describe_counter!(
        REQUESTS_TOTAL,
        "Decision requests by provider and outcome (ok | invalid | busy | auth | rejected | overloaded | upstream | transport | malformed | unknown_choice)"
    );
    metrics::describe_histogram!(
        DURATION_SECONDS,
        metrics::Unit::Seconds,
        "Time to answer a decision, including queueing and provider retries"
    );
    metrics::describe_counter!(BAND_TOTAL, "Answered decisions by provider and band");
    metrics::describe_counter!(
        INPUT_TOKENS_TOTAL,
        "Input tokens the decision provider billed"
    );
    metrics::describe_counter!(
        OUTPUT_TOKENS_TOTAL,
        "Output tokens the decision provider billed"
    );
    metrics::describe_counter!(
        PROVIDER_RETRIES_TOTAL,
        "Decision provider calls retried, by the HTTP status that caused the retry"
    );
}

pub(crate) fn record_outcome(provider: &'static str, outcome: &'static str, elapsed: Duration) {
    metrics::counter!(REQUESTS_TOTAL, "provider" => provider, "outcome" => outcome).increment(1);
    metrics::histogram!(DURATION_SECONDS, "provider" => provider).record(elapsed.as_secs_f64());
}

/// Counted under provider `none`, with no duration: nothing was attempted.
pub(crate) fn record_unconfigured(outcome: &'static str) {
    metrics::counter!(REQUESTS_TOTAL, "provider" => "none", "outcome" => outcome).increment(1);
}

pub(crate) fn record_answer(provider: &'static str, band: DecisionBand, usage: DecisionUsage) {
    metrics::counter!(BAND_TOTAL, "provider" => provider, "band" => band.as_str()).increment(1);
    metrics::counter!(INPUT_TOKENS_TOTAL, "provider" => provider).increment(usage.input_tokens);
    metrics::counter!(OUTPUT_TOKENS_TOTAL, "provider" => provider).increment(usage.output_tokens);
}

pub(crate) fn record_retry(provider: &'static str, status: u16) {
    metrics::counter!(PROVIDER_RETRIES_TOTAL, "provider" => provider, "status" => status.to_string())
        .increment(1);
}
