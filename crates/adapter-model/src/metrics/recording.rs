//! Prometheus counters for adapter webhook ingress and inbound binding routing.
//!
//! `loop_id` and `connection_id` are labels, passed as `""` when unknown.
//! `event_type` is the provider's event name (bounded by its API): GitHub's
//! `X-GitHub-Event` header, or Slack's payload `type` / `"slash_command"`.
//!
//! Cardinality: loops × connections × adapters × outcomes × reasons ×
//! `event_types` — keep `reason` small and prune unused loops/connections.

pub(crate) const ADAPTER_WEBHOOK_TOTAL: &str = "moatless_adapter_webhook_total";

/// Outbound adapter API call outcomes. Low cardinality: adapter × outcome,
/// where outcome is one of `success` / `auth_error` / `error`. Safe per the
/// module doc's cardinality note.
pub(crate) const ADAPTER_API_TOTAL: &str = "moatless_adapter_api_total";

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
}

/// Record one adapter webhook outcome.
///
/// `outcome` is one of `"success"`, `"ignored"`, `"error"`; `reason`,
/// `event_type`, `loop_id`, and `connection_id` pass `""` when not yet known.
pub fn record_webhook(
    adapter: &'static str,
    outcome: &'static str,
    reason: &'static str,
    event_type: &str,
    loop_id: &str,
    connection_id: &str,
) {
    metrics::counter!(
        ADAPTER_WEBHOOK_TOTAL,
        "adapter" => adapter,
        "outcome" => outcome,
        "reason" => reason,
        "event_type" => event_type.to_string(),
        "loop_id" => loop_id.to_string(),
        "connection_id" => connection_id.to_string(),
    )
    .increment(1);
}

/// Record one outbound adapter API call outcome.
///
/// `outcome` is one of `"success"`, `"auth_error"`, `"error"`. A 401 healed by
/// a reactive token refresh records `auth_error` then `success` — the signal
/// the auth-failure alert keys off.
pub fn record_api_call(adapter: &'static str, outcome: &'static str) {
    metrics::counter!(
        ADAPTER_API_TOTAL,
        "adapter" => adapter,
        "outcome" => outcome,
    )
    .increment(1);
}

/// Inbound binding routing outcomes, by provider, loop, connection, and reason.
///
/// `describe_binding_metrics` in `moatless-loops` registers the description.
pub const BINDING_INBOUND_TOTAL: &str = "moatless_binding_inbound_total";

/// Record one inbound routing outcome.
pub fn record_inbound(
    provider: &str,
    outcome: &'static str,
    reason: &'static str,
    loop_id: &str,
    connection_id: &str,
) {
    metrics::counter!(
        BINDING_INBOUND_TOTAL,
        "provider" => provider.to_string(),
        "outcome" => outcome,
        "reason" => reason,
        "loop_id" => loop_id.to_string(),
        "connection_id" => connection_id.to_string(),
    )
    .increment(1);
}
