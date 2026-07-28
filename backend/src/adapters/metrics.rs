//! Prometheus counters for adapter webhook ingress.
//!
//! `trigger_id` and `connection_id` are emitted as labels so the admin UI
//! can render per-trigger and per-connection counts. Pass empty strings
//! at call sites where the IDs aren't yet known (e.g. before rule lookup).
//!
//! `event_type` is the provider's own event name — for GitHub it's the
//! `X-GitHub-Event` header (`check_run`, `pull_request`, ...); for Slack
//! it's the outer payload `type` (`event_callback`, `url_verification`)
//! plus the synthetic `"slash_command"` for slash-command deliveries. The
//! vocabulary is bounded by each provider's API so cardinality is safe.
//!
//! Cardinality note: number of active triggers × connections × adapters ×
//! outcomes × reasons × event_types. Keep the `reason` vocabulary small
//! and prune unused triggers/connections to avoid runaway series in
//! Prometheus.

pub(crate) const ADAPTER_WEBHOOK_TOTAL: &str = "moatless_adapter_webhook_total";

/// Outbound adapter API call outcomes. Low cardinality: adapter × outcome,
/// where outcome is one of `success` / `auth_error` / `error`. Safe per the
/// module doc's cardinality note.
pub(crate) const ADAPTER_API_TOTAL: &str = "moatless_adapter_api_total";

/// Outcome of claiming the event a Task's own shell command created. Low
/// cardinality: adapter × shape × outcome × reason, all fixed vocabularies.
pub(crate) const ADAPTER_SHELL_CLAIM_TOTAL: &str = "moatless_adapter_shell_claim_total";

/// Register adapter metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_adapter_metrics() {
    metrics::describe_counter!(
        ADAPTER_WEBHOOK_TOTAL,
        "Adapter webhook outcomes (success | ignored | error) by adapter, trigger, connection, reason, and event_type"
    );
    metrics::describe_counter!(
        ADAPTER_API_TOTAL,
        "Outbound adapter API call outcomes (success | auth_error | error) by adapter and outcome"
    );
    metrics::describe_counter!(
        ADAPTER_SHELL_CLAIM_TOTAL,
        "Shell-reconciliation claim outcomes (claimed | unclaimed) by adapter, posting shape, and reason"
    );
}

/// Record one adapter webhook outcome.
///
/// `outcome` must be one of `"success"`, `"ignored"`, `"error"`.
/// `reason` is a fixed-vocabulary label (e.g. `"inactive"`, `"no_trigger"`,
/// `"delivery_failed"`); pass `""` when not applicable.
/// `event_type` is the provider's event name (see module doc); pass `""`
/// when the request errored before the type could be determined.
/// `trigger_id` / `connection_id` are passed as empty strings when not
/// resolved yet (e.g. early validation failures).
pub(crate) fn record_webhook(
    adapter: &'static str,
    outcome: &'static str,
    reason: &'static str,
    event_type: &str,
    trigger_id: &str,
    connection_id: &str,
) {
    metrics::counter!(
        ADAPTER_WEBHOOK_TOTAL,
        "adapter" => adapter,
        "outcome" => outcome,
        "reason" => reason,
        "event_type" => event_type.to_string(),
        "trigger_id" => trigger_id.to_string(),
        "connection_id" => connection_id.to_string(),
    )
    .increment(1);
}

/// Record one outbound adapter API call outcome.
///
/// `outcome` must be one of `"success"`, `"auth_error"`, `"error"`. A 401 that
/// is healed by a reactive token refresh records `auth_error` then `success` —
/// exactly the signal the auth-failure alert keys off.
pub(crate) fn record_api_call(adapter: &'static str, outcome: &'static str) {
    metrics::counter!(
        ADAPTER_API_TOTAL,
        "adapter" => adapter,
        "outcome" => outcome,
    )
    .increment(1);
}

/// Record one shell-reconciliation claim outcome.
///
/// `shape` names the posting shape the command was recognised as (e.g.
/// `"pr_create"`, `"api_review_comment"`). `outcome` is `"claimed"` when the
/// Task's own event was recorded as its own, `"unclaimed"` when the claim was
/// abandoned — an abandoned claim means the event comes back to the Task as an
/// echo, so the counter is the only warning an operator gets. `reason` is a
/// fixed-vocabulary label; pass `""` when not applicable.
pub(crate) fn record_shell_claim(
    adapter: &'static str,
    shape: &'static str,
    outcome: &'static str,
    reason: &'static str,
) {
    metrics::counter!(
        ADAPTER_SHELL_CLAIM_TOTAL,
        "adapter" => adapter,
        "shape" => shape,
        "outcome" => outcome,
        "reason" => reason,
    )
    .increment(1);
}
