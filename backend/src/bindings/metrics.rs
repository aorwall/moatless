//! Prometheus counters for binding inbound routing and outbound send.
//!
//! `trigger_id` and `connection_id` are emitted as labels so the admin UI
//! can render per-trigger / per-connection counts.

pub(crate) const BINDING_INBOUND_TOTAL: &str = "moatless_binding_inbound_total";
pub(crate) const BINDING_OUTBOUND_TOTAL: &str = "moatless_binding_outbound_total";

/// Register binding metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_binding_metrics() {
    metrics::describe_counter!(
        BINDING_INBOUND_TOTAL,
        "Inbound binding routing outcomes (success | ignored | error) by provider, trigger, connection, and reason"
    );
    metrics::describe_counter!(
        BINDING_OUTBOUND_TOTAL,
        "Outbound binding send outcomes (success | ignored | error) by provider, trigger, connection, and reason"
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
    /// No trigger matched the event's source.
    pub(crate) const NO_RULE_MATCH: &str = "no_rule_match";
    /// The matched trigger belongs to a different connection.
    pub(crate) const WRONG_CONNECTION: &str = "wrong_connection";
    /// The trigger is deactivated.
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
    trigger_id: &str,
    connection_id: &str,
) {
    metrics::counter!(
        BINDING_INBOUND_TOTAL,
        "provider" => provider.to_string(),
        "outcome" => outcome,
        "reason" => reason,
        "trigger_id" => trigger_id.to_string(),
        "connection_id" => connection_id.to_string(),
    )
    .increment(1);
}

/// Record one outbound send outcome.
pub(crate) fn record_outbound(
    provider: &str,
    outcome: &'static str,
    reason: &'static str,
    trigger_id: &str,
    connection_id: &str,
) {
    metrics::counter!(
        BINDING_OUTBOUND_TOTAL,
        "provider" => provider.to_string(),
        "outcome" => outcome,
        "reason" => reason,
        "trigger_id" => trigger_id.to_string(),
        "connection_id" => connection_id.to_string(),
    )
    .increment(1);
}

/// `#[cfg(test)]` capture of what a code path actually wrote to
/// [`BINDING_INBOUND_TOTAL`].
///
/// A routing outcome nobody can observe is the fault this counter exists to
/// remove, so "the reason was recorded" is worth asserting on the counter
/// itself rather than on a value the code happened to compute on the way there.
#[cfg(test)]
pub(crate) mod test_support {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    /// One inbound counter increment, flattened to the labels callers assert on.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct InboundSample {
        pub(crate) provider: String,
        pub(crate) outcome: String,
        pub(crate) reason: String,
        pub(crate) trigger_id: String,
        pub(crate) connection_id: String,
        pub(crate) count: u64,
    }

    /// Run `f` against a recorder installed for this thread only, and return
    /// every inbound-counter increment it produced.
    ///
    /// Thread-local rather than global: the test binary runs these in parallel
    /// with everything else that meters, and a global recorder would mix them.
    /// `f` must therefore not `.await` across a thread hop — pass it the
    /// synchronous part of the path under test.
    pub(crate) fn captured_inbound(f: impl FnOnce()) -> Vec<InboundSample> {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, f);

        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, _, _, _)| key.key().name() == super::BINDING_INBOUND_TOTAL)
            .map(|(key, _, _, value)| {
                let label = |name: &str| {
                    key.key()
                        .labels()
                        .find(|l| l.key() == name)
                        .map_or_else(String::new, |l| l.value().to_string())
                };
                InboundSample {
                    provider: label("provider"),
                    outcome: label("outcome"),
                    reason: label("reason"),
                    trigger_id: label("trigger_id"),
                    connection_id: label("connection_id"),
                    count: match value {
                        DebugValue::Counter(count) => count,
                        _ => 0,
                    },
                }
            })
            .collect()
    }
}
