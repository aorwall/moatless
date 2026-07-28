//! Durable execution of inbound chat-message routing.
//!
//! Routing an inbound message creates a task and provisions a sandbox, which
//! is far more than a chat provider will wait for on its webhook response
//! deadline. The webhook handler therefore hands the routing over to
//! [`crate::adapters::work`] before it acknowledges, and this module is the
//! other end of that handover: the payload that survives the admitting
//! process, and the handler the worker runs against it.
//!
//! The handler is provider-agnostic because the work is — every chat provider
//! ends at the same [`route_inbound_message`] call with the same arguments.
//! Each provider still owns its own `kind` constant and its own registration,
//! so a migration adds a module and a registration line and never edits a
//! shared match arm.
//!
//! [`route_inbound_message`]: crate::bindings::runtime::route_inbound_message

use moatless_backend_core::scope::ScopeContext;
use serde::{Deserialize, Serialize};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::adapters::models::{AdapterConnection, InboundMessage, SubscriptionMatch};
use crate::adapters::work::{
    AdapterWork, AdapterWorkFailure, AdapterWorkHandler, AdapterWorkRequest, Admission,
};
use crate::state::AppState;

/// Everything [`route_inbound_message`] needs, in a form that outlives the
/// process that admitted it.
///
/// The connection and the trigger are referenced by id rather than embedded:
/// they are durable rows already, and a connection carries credentials that
/// have no business being copied into a queue row that outlives the request by
/// days.
///
/// Fields may only ever be *added*, and only with `#[serde(default)]`. During a
/// rolling deploy the new build executes records the old build admitted, so a
/// payload that stops deserializing is a payload whose work goes terminal.
///
/// [`route_inbound_message`]: crate::bindings::runtime::route_inbound_message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct InboundWorkPayload {
    pub(crate) connection_id: String,
    pub(crate) rule_id: String,
    pub(crate) subject_kind: String,
    pub(crate) subject_key: String,
    pub(crate) destination_config: serde_json::Value,
    /// The provider's own event name, carried only so the webhook outcome
    /// metric keeps the labels it had while routing ran on the request path.
    /// Empty for providers that never labelled it.
    pub(crate) event_type: String,
    pub(crate) message: InboundMessage,
}

/// The connection or trigger a record was admitted against is gone.
///
/// A synthesized condition, not a wrapped failure: there is no underlying
/// error to retain, and no retry can bring the row back.
#[derive(Debug, thiserror::Error)]
#[error("the {what} (`{id}`) this work was admitted for no longer exists")]
struct Vanished {
    what: &'static str,
    id: String,
}

/// Routes one admitted inbound message.
///
/// Idempotent as the seam requires, and not by its own doing: repeat delivery
/// to a binding is stopped by the event-receipt table inside
/// [`route_inbound_message`], and repeat task creation by the binding that
/// creation upserts. A re-execution after a crash therefore re-walks the same
/// bindings and delivers nothing twice.
///
/// [`route_inbound_message`]: crate::bindings::runtime::route_inbound_message
pub(crate) struct InboundMessageHandler {
    /// Provider label used for routing, bindings, and metrics.
    provider: &'static str,
    state: AppState,
}

impl InboundMessageHandler {
    pub(crate) fn new(provider: &'static str, state: AppState) -> Self {
        Self { provider, state }
    }
}

#[async_trait::async_trait]
impl AdapterWorkHandler for InboundMessageHandler {
    async fn execute(&self, work: &AdapterWork) -> Result<(), AdapterWorkFailure> {
        let payload: InboundWorkPayload =
            serde_json::from_value(work.payload.clone()).map_err(AdapterWorkFailure::permanent)?;
        let InboundWorkPayload {
            connection_id,
            rule_id,
            subject_kind,
            subject_key,
            destination_config,
            event_type,
            message,
        } = payload;

        let (connection, rule) = resolve(&self.state, &connection_id, &rule_id).await?;

        let outcome = crate::bindings::runtime::route_inbound_message(
            &self.state,
            &connection,
            &rule,
            message,
            self.provider,
            &subject_kind,
            &subject_key,
            destination_config,
        )
        .await;

        // The webhook outcome metric stays this path's responsibility. The
        // seam's own execution metric is `{provider, kind, outcome}` and is
        // deliberately low-cardinality, so dropping this call would lose the
        // per-trigger and per-connection failure visibility the spawned path
        // had.
        match outcome {
            Ok(_) => {
                record_webhook(
                    self.provider,
                    "success",
                    "",
                    &event_type,
                    &rule.id,
                    &connection.id,
                );
                Ok(())
            }
            Err(e) => {
                record_webhook(
                    self.provider,
                    "error",
                    "delivery_failed",
                    &event_type,
                    &rule.id,
                    &connection.id,
                );
                Err(e.into())
            }
        }
    }
}

/// Re-read the connection and trigger the work was admitted against.
///
/// Read at execution time rather than captured at admission so the work runs
/// against the current configuration, and so a deleted trigger ends the record
/// instead of routing through a stale copy of itself.
async fn resolve(
    state: &AppState,
    connection_id: &str,
    rule_id: &str,
) -> Result<(AdapterConnection, SubscriptionMatch), AdapterWorkFailure> {
    // reason: durable adapter work runs in the worker with no user session;
    // the connection is named by an already-admitted record, not by caller
    // input.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let connection = PgAdapterConnectionRepository(state.db.clone())
        .find_by_id(connection_id, &sys_ctx)
        .await
        .map_err(AdapterWorkFailure::retryable)?
        .ok_or_else(|| {
            AdapterWorkFailure::permanent(Vanished {
                what: "adapter connection",
                id: connection_id.to_string(),
            })
        })?;

    let rule = state
        .loops
        .find_subscription_by_id(rule_id)
        .await
        .map_err(AdapterWorkFailure::retryable)?
        .ok_or_else(|| {
            AdapterWorkFailure::permanent(Vanished {
                what: "trigger",
                id: rule_id.to_string(),
            })
        })?;

    Ok((connection, rule))
}

/// Take durable ownership of one inbound message, keyed by the provider's own
/// delivery id.
///
/// Returning `Ok` is the point the caller may acknowledge its provider:
/// [`Admission::Admitted`] means the routing is owned by the backend,
/// [`Admission::Duplicate`] means it already was. An `Err` must reach the
/// provider as a failure — the delivery is still theirs, and swallowing it into
/// a 200 is exactly the lost-webhook bug this path exists to remove.
pub(crate) async fn admit_inbound_message(
    state: &AppState,
    provider: &'static str,
    kind: &'static str,
    event_id: String,
    payload: InboundWorkPayload,
) -> Result<Admission, AdapterError> {
    let payload = serde_json::to_value(payload).map_err(|e| {
        AdapterError::Internal(format!("Failed to serialize inbound work payload: {e}"))
    })?;
    let admission = state
        .adapter_work
        .admit(AdapterWorkRequest {
            provider,
            kind,
            event_id,
            payload,
        })
        .await?;
    Ok(admission)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::models::Audience;
    use crate::adapters::AdapterKind;

    fn payload() -> InboundWorkPayload {
        InboundWorkPayload {
            connection_id: "pc_1".into(),
            rule_id: "tt_1".into(),
            subject_kind: "slack_thread".into(),
            subject_key: "T1:C1:1700000000.000100".into(),
            destination_config: serde_json::json!({ "channel": "C1" }),
            event_type: "event_callback".into(),
            message: InboundMessage {
                provider: AdapterKind::Slack,
                source_id: "C1".into(),
                thread_id: Some("1700000000.000100".into()),
                message_thread_id: "1700000000.000100".into(),
                user_id: "user-1".into(),
                text: "hello".into(),
                file_ids: vec!["f1".into()],
                thread_properties: serde_json::json!({ "channel": "C1" }),
                provider_context: serde_json::json!({ "is_bot_mentioned": true }),
                external_event_id: Some("slack:ts:1700000000.000100".into()),
                source_task_id: None,
                audience: Audience::Everyone,
            },
        }
    }

    /// The payload is the whole of what survives the admitting process, so a
    /// field that does not round-trip is a field the handler silently loses.
    #[test]
    fn the_payload_round_trips_through_the_queue_row() {
        let stored = serde_json::to_value(payload()).expect("payload serializes");
        let read: InboundWorkPayload = serde_json::from_value(stored).expect("payload reads back");

        let original = payload();
        assert_eq!(read.connection_id, original.connection_id);
        assert_eq!(read.rule_id, original.rule_id);
        assert_eq!(read.subject_kind, original.subject_kind);
        assert_eq!(read.subject_key, original.subject_key);
        assert_eq!(read.destination_config, original.destination_config);
        assert_eq!(read.event_type, original.event_type);
        assert_eq!(read.message.provider, AdapterKind::Slack);
        assert_eq!(read.message.source_id, original.message.source_id);
        assert_eq!(read.message.thread_id, original.message.thread_id);
        assert_eq!(
            read.message.message_thread_id,
            original.message.message_thread_id
        );
        assert_eq!(read.message.user_id, original.message.user_id);
        assert_eq!(read.message.text, original.message.text);
        assert_eq!(read.message.file_ids, original.message.file_ids);
        assert_eq!(
            read.message.thread_properties,
            original.message.thread_properties
        );
        assert_eq!(
            read.message.provider_context,
            original.message.provider_context
        );
        assert_eq!(
            read.message.external_event_id,
            original.message.external_event_id
        );
        assert_eq!(read.message.source_task_id, original.message.source_task_id);
    }

    /// A payload the handler cannot read can never be read, so it must not
    /// spend eight attempts proving that.
    #[test]
    fn an_unreadable_payload_is_permanent() {
        let failure = serde_json::from_value::<InboundWorkPayload>(serde_json::json!({}))
            .map_err(AdapterWorkFailure::permanent)
            .expect_err("an empty object is not a payload");
        assert!(matches!(failure, AdapterWorkFailure::Permanent(_)));
    }

    #[test]
    fn a_backend_failure_is_retried_and_keeps_its_cause() {
        let failure = AdapterWorkFailure::from(AdapterError::Db(sqlx::Error::PoolTimedOut));
        assert!(matches!(failure, AdapterWorkFailure::Retryable(_)));
        let source = std::error::Error::source(&failure).expect("the routing error is retained");
        assert!(source.to_string().contains("Database error"));

        let failure = AdapterWorkFailure::from(AdapterError::Internal("sandbox refused".into()));
        assert!(matches!(failure, AdapterWorkFailure::Retryable(_)));
    }

    #[test]
    fn a_failure_that_describes_the_event_is_terminal() {
        // `route_inbound_message` returns this both for a deactivated trigger
        // and for a subject with nothing left to deliver to. Neither improves
        // by being tried again.
        let failure = AdapterWorkFailure::from(AdapterError::ConfigNotFound("tt_1".into()));
        assert!(matches!(failure, AdapterWorkFailure::Permanent(_)));
        let source = std::error::Error::source(&failure).expect("the routing error is retained");
        assert!(source.to_string().contains("tt_1"));
    }

    #[test]
    fn a_vanished_row_names_what_is_missing() {
        let failure = AdapterWorkFailure::permanent(Vanished {
            what: "trigger",
            id: "tt_gone".into(),
        });
        let source = std::error::Error::source(&failure).expect("the cause is retained");
        let rendered = source.to_string();
        assert!(rendered.contains("trigger"), "{rendered}");
        assert!(rendered.contains("tt_gone"), "{rendered}");
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions; the lints only fire when the db-tests feature is enabled"
)]
mod db_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use sqlx::PgPool;
    use tokio_util::sync::CancellationToken;

    use crate::adapters::work::{
        spawn_adapter_work_worker, AdapterWorkHandlers, AdapterWorkPolicy, AdapterWorkRequest,
        AdapterWorkStore,
    };

    use super::*;

    const KIND: &str = "slack.inbound_message";

    /// Fails the way `route_inbound_message` fails when the backend, rather
    /// than the event, is at fault — a database blip mid-routing — and
    /// classifies it through the same `From<AdapterError>` the real handler uses.
    /// The point is not that this stand-in fails, it is that *this
    /// classification* is what the shared worker policy retries.
    struct FailingRoute {
        runs: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl AdapterWorkHandler for FailingRoute {
        async fn execute(&self, _work: &AdapterWork) -> Result<(), AdapterWorkFailure> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Err(AdapterWorkFailure::from(AdapterError::Internal(
                "Failed to create task: sandbox provisioning timed out".into(),
            )))
        }
    }

    /// Verification: an injected route failure is retried. The record spends
    /// its budget one attempt at a time, keeps its cause, and only then goes
    /// terminal — instead of the spawned path's single silent `warn!`.
    #[sqlx::test]
    async fn an_injected_route_failure_is_retried_then_recorded(pool: PgPool) {
        let store = Arc::new(AdapterWorkStore::with_policy(
            pool,
            AdapterWorkPolicy {
                max_attempts: 3,
                base_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(1),
                poll_interval: Duration::from_millis(5),
                maintenance_interval: Duration::from_millis(5),
                ..AdapterWorkPolicy::default()
            },
        ));
        store
            .admit(AdapterWorkRequest {
                provider: "slack",
                kind: KIND,
                event_id: "Ev-route-fail".to_string(),
                payload: serde_json::json!({}),
            })
            .await
            .unwrap();

        let runs = Arc::new(AtomicUsize::new(0));
        let mut handlers = AdapterWorkHandlers::new();
        handlers.register(
            KIND,
            Arc::new(FailingRoute {
                runs: Arc::clone(&runs),
            }),
        );

        let cancel = CancellationToken::new();
        let handle =
            spawn_adapter_work_worker(Arc::clone(&store), Arc::new(handlers), cancel.clone());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline && runs.load(Ordering::SeqCst) < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
        handle.await.expect("worker exits cleanly on cancellation");

        assert_eq!(
            runs.load(Ordering::SeqCst),
            3,
            "the failure was retried until the budget ran out, not dropped"
        );
        let (status, attempts, last_error) = store
            .peek("slack", KIND, "Ev-route-fail")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(attempts, 3);
        let last_error = last_error.expect("an abandoned webhook says why");
        assert!(
            last_error.contains("sandbox provisioning timed out"),
            "the routing cause survives onto the record: {last_error}"
        );
    }
}
