//! Prometheus counters for binding inbound routing and outbound send.
//!
//! `loop_id` and `connection_id` are emitted as labels so the admin UI
//! can render per-loop / per-connection counts.

pub(crate) const BINDING_INBOUND_TOTAL: &str = "moatless_binding_inbound_total";
pub(crate) const BINDING_OUTBOUND_TOTAL: &str = "moatless_binding_outbound_total";

/// Register binding metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_binding_metrics() {
    metrics::describe_counter!(
        BINDING_INBOUND_TOTAL,
        "Inbound binding routing outcomes (success | ignored | error) by provider, loop, connection, and reason"
    );
    metrics::describe_counter!(
        BINDING_OUTBOUND_TOTAL,
        "Outbound binding send outcomes (success | ignored | error) by provider, loop, connection, and reason"
    );
}

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
pub(crate) mod inbound_reason {
    /// No adapter connection matched the event's scope.
    pub(crate) const NO_CONNECTION: &str = "no_connection";
    /// No subscription matched the event's source.
    pub(crate) const NO_RULE_MATCH: &str = "no_rule_match";
    /// The matched subscription belongs to a different connection.
    pub(crate) const WRONG_CONNECTION: &str = "wrong_connection";
    /// The subscription is deactivated.
    pub(crate) const INACTIVE_RULE: &str = "inactive_rule";
    /// The binding's own task authored the event.
    pub(crate) const SELF_ECHO: &str = "self_echo";
    /// The binding already received this event.
    pub(crate) const DUPLICATE_RECEIPT: &str = "duplicate_receipt";
    /// The audience named someone else, so this binding holds nothing.
    pub(crate) const NOT_IN_AUDIENCE: &str = "not_in_audience";
    /// Every binding on a subject that had some was passed over, so the event
    /// reached no task at all. The one outcome that is nobody's correct answer.
    pub(crate) const NO_RECIPIENTS: &str = "no_recipients";
    /// Delivered to a task the event itself created.
    pub(crate) const NEW_TASK: &str = "new_task";
    /// Delivered to a task that was already bound.
    pub(crate) const EXISTING_TASK: &str = "existing_task";
    /// Handing the message to the task message flow failed.
    pub(crate) const DELIVER_FAILED: &str = "deliver_failed";
}

/// Record one inbound routing outcome.
pub(crate) fn record_inbound(
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

/// Record one outbound send outcome.
pub(crate) fn record_outbound(
    provider: &str,
    outcome: &'static str,
    reason: &'static str,
    loop_id: &str,
    connection_id: &str,
) {
    metrics::counter!(
        BINDING_OUTBOUND_TOTAL,
        "provider" => provider.to_string(),
        "outcome" => outcome,
        "reason" => reason,
        "loop_id" => loop_id.to_string(),
        "connection_id" => connection_id.to_string(),
    )
    .increment(1);
}
