//! Prometheus counters for adapter webhook ingress and inbound binding routing.
//!
//! `loop_id` and `connection_id` are emitted as labels so the admin UI
//! can render per-loop and per-connection counts. Pass empty strings
//! at call sites where the IDs aren't yet known (e.g. before subscription lookup).
//!
//! `event_type` is the provider's own event name — for GitHub it's the
//! `X-GitHub-Event` header (`check_run`, `pull_request`, ...); for Slack
//! it's the outer payload `type` (`event_callback`, `url_verification`)
//! plus the synthetic `"slash_command"` for slash-command deliveries. The
//! vocabulary is bounded by each provider's API so cardinality is safe.
//!
//! Cardinality note: number of active loops × connections × adapters ×
//! outcomes × reasons × `event_types`. Keep the `reason` vocabulary small
//! and prune unused loops/connections to avoid runaway series in
//! Prometheus.

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
/// `outcome` must be one of `"success"`, `"ignored"`, `"error"`.
/// `reason` is a fixed-vocabulary label (e.g. `"inactive"`, `"no_trigger"`,
/// `"delivery_failed"`); pass `""` when not applicable.
/// `event_type` is the provider's event name (see module doc); pass `""`
/// when the request errored before the type could be determined.
/// `loop_id` / `connection_id` are passed as empty strings when not
/// resolved yet (e.g. early validation failures).
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
/// `outcome` must be one of `"success"`, `"auth_error"`, `"error"`. A 401 that
/// is healed by a reactive token refresh records `auth_error` then `success` —
/// exactly the signal the auth-failure alert keys off.
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
/// `describe_binding_metrics` in `moatless-loops` registers the description,
/// beside the outbound counter that belongs to that band alone.
pub const BINDING_INBOUND_TOTAL: &str = "moatless_binding_inbound_total";

/// The `reason` labels the shared router emits on [`BINDING_INBOUND_TOTAL`].
///
/// A reason is a word an operator queries by, and there is one router writing
/// them, so one query answers "why did events stop reaching tasks" across every
/// adapter.
///
/// The other distinction the names carry is whether the event is *accounted
/// for* by the skip. A task that wrote the event, or that has already received
/// it, has it — nothing was lost by passing that binding over. A binding the
/// audience did not name holds nothing, so a subject whose every binding was
/// skipped for that reason has delivered the event to no task at all, which is
/// the failure this vocabulary exists to make visible.
pub mod inbound_reason {
    /// No adapter connection matched the event's scope.
    pub const NO_CONNECTION: &str = "no_connection";
    /// No subscription matched the event's source.
    pub const NO_RULE_MATCH: &str = "no_rule_match";
    /// The matched subscription belongs to a different connection.
    pub const WRONG_CONNECTION: &str = "wrong_connection";
    /// The subscription is deactivated.
    pub const INACTIVE_RULE: &str = "inactive_rule";
    /// The binding's own task authored the event.
    pub const SELF_ECHO: &str = "self_echo";
    /// The binding already received this event.
    pub const DUPLICATE_RECEIPT: &str = "duplicate_receipt";
    /// Another delivery holds the creation claim for this subject, so this one
    /// retries into the task that claim produces. Expected whenever a sender
    /// emits one message as two events.
    pub const CREATION_CLAIMED: &str = "creation_claimed";
    /// The audience named someone else, so this binding holds nothing.
    pub const NOT_IN_AUDIENCE: &str = "not_in_audience";
    /// Every binding on a subject that had some was passed over, so the event
    /// reached no task at all. The one outcome that is nobody's correct answer.
    pub const NO_RECIPIENTS: &str = "no_recipients";
    /// Delivered to a task the event itself created.
    pub const NEW_TASK: &str = "new_task";
    /// Delivered to a task that was already bound.
    pub const EXISTING_TASK: &str = "existing_task";
    /// Handing the message to the task message flow failed.
    pub const DELIVER_FAILED: &str = "deliver_failed";
}

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
