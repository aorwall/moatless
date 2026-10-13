//! Durable execution of inbound chat-message routing.
//!
//! Routing creates a task and provisions a sandbox — too slow for a chat
//! webhook deadline — so the handler hands off to
//! [`crate::adapters::delivery`] before acknowledging; this is the
//! worker-side handler for the surviving payload, built by
//! [`inbound_delivery_request`][idr]. Adapter-agnostic: every chat Adapter
//! owns only its `kind` constant and registration.
//!
//! [idr]: moatless_adapter_model::delivery::inbound_delivery_request

use std::sync::Arc;

use crate::bindings::runtime::RoutingService;
use moatless_adapter_model::delivery::{
    AdapterDelivery, AdapterDeliveryFailure, AdapterDeliveryHandler,
};
use moatless_adapter_model::metrics::record_webhook;
use moatless_types::adapter::InboundDeliveryPayload;

/// Routes one admitted inbound message.
///
/// Idempotent only via its callees: the event-receipt table inside
/// [`route_inbound_message`][rim] stops repeat delivery, and the binding it
/// upserts stops repeat task creation — so a crash replays safely.
///
/// [rim]: crate::bindings::runtime::route_inbound_message
pub struct InboundMessageHandler {
    /// Sender label used for routing, bindings, and metrics.
    adapter: &'static str,
    routing: Arc<RoutingService>,
}

impl InboundMessageHandler {
    pub fn new(adapter: &'static str, routing: Arc<RoutingService>) -> Self {
        Self { adapter, routing }
    }
}

#[async_trait::async_trait]
impl AdapterDeliveryHandler for InboundMessageHandler {
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
        let payload: InboundDeliveryPayload = serde_json::from_value(delivery.payload.clone())
            .map_err(AdapterDeliveryFailure::permanent)?;
        let InboundDeliveryPayload {
            connection_id,
            loop_id,
            subject_kind,
            subject_key,
            destination_config,
            event_type,
            message,
        } = payload;

        let (connection, subscription) = self
            .routing
            .resolve_connection_and_subscription(&connection_id, &loop_id)
            .await?;

        let outcome = self
            .routing
            .route_inbound_message(
                &connection,
                &subscription,
                message,
                self.adapter,
                &subject_kind,
                &subject_key,
                destination_config,
            )
            .await;

        // The webhook outcome metric stays this path's responsibility. The
        // seam's own execution metric is `{adapter, kind, outcome}` and is
        // deliberately low-cardinality, so dropping this call would lose the
        // per-Loop and per-connection failure visibility the spawned path
        // had.
        match outcome {
            Ok(_) => {
                record_webhook(
                    self.adapter,
                    "success",
                    "",
                    &event_type,
                    &subscription.id,
                    &connection.id,
                );
                Ok(())
            }
            Err(e) => {
                record_webhook(
                    self.adapter,
                    "error",
                    "delivery_failed",
                    &event_type,
                    &subscription.id,
                    &connection.id,
                );
                Err(e.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
    use moatless_types::adapter::AdapterKind;
    use moatless_types::adapter::{Audience, InboundMessage};

    fn payload() -> InboundDeliveryPayload {
        InboundDeliveryPayload {
            connection_id: "pc_1".into(),
            loop_id: "tt_1".into(),
            subject_kind: "slack_thread".into(),
            subject_key: "T1:C1:1700000000.000100".into(),
            destination_config: serde_json::json!({ "channel": "C1" }),
            event_type: "event_callback".into(),
            message: InboundMessage {
                adapter_kind: AdapterKind::Slack,
                source_id: "C1".into(),
                thread_id: Some("1700000000.000100".into()),
                message_thread_id: "1700000000.000100".into(),
                user_id: "user-1".into(),
                author: Some("@ada".into()),
                sender_external_id: None,
                text: "hello".into(),
                file_ids: vec!["f1".into()],
                thread_properties: serde_json::json!({ "channel": "C1" }),
                adapter_context: serde_json::json!({ "is_bot_mentioned": true }),
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
        let read: InboundDeliveryPayload =
            serde_json::from_value(stored).expect("payload reads back");

        let original = payload();
        assert_eq!(read.connection_id, original.connection_id);
        assert_eq!(read.loop_id, original.loop_id);
        assert_eq!(read.subject_kind, original.subject_kind);
        assert_eq!(read.subject_key, original.subject_key);
        assert_eq!(read.destination_config, original.destination_config);
        assert_eq!(read.event_type, original.event_type);
        assert_eq!(read.message.adapter_kind, AdapterKind::Slack);
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
            read.message.adapter_context,
            original.message.adapter_context
        );
        assert_eq!(
            read.message.external_event_id,
            original.message.external_event_id
        );
        assert_eq!(read.message.source_task_id, original.message.source_task_id);
        assert_eq!(read.message.author, original.message.author);
    }

    /// A record the previous build wrote, executed by this one mid-deploy. It
    /// names no author, and it still admits: nothing about the delivery turns
    /// on knowing who wrote it.
    #[test]
    fn a_payload_written_before_the_author_was_captured_still_reads() {
        let mut stored = serde_json::to_value(payload()).expect("payload serializes");
        stored["message"]
            .as_object_mut()
            .expect("the message is an object")
            .remove("author");

        let read: InboundDeliveryPayload =
            serde_json::from_value(stored).expect("payload reads back");
        assert_eq!(read.message.author, None);
        assert_eq!(read.message.text, "hello");
    }

    /// A payload the handler cannot read can never be read, so it must not
    /// spend eight attempts proving that.
    #[test]
    fn an_unreadable_payload_is_permanent() {
        let failure = serde_json::from_value::<InboundDeliveryPayload>(serde_json::json!({}))
            .map_err(AdapterDeliveryFailure::permanent)
            .expect_err("an empty object is not a payload");
        assert!(matches!(failure, AdapterDeliveryFailure::Permanent(_)));
    }

    #[test]
    fn a_backend_failure_is_retried_and_keeps_its_cause() {
        let failure = AdapterDeliveryFailure::from(AdapterError::from(SharedAdapterError::Store(
            moatless_types::store::StoreError::with_source(
                "read the connection",
                sqlx::Error::PoolTimedOut,
            ),
        )));
        assert!(matches!(failure, AdapterDeliveryFailure::Retryable(_)));
        let source = std::error::Error::source(&failure).expect("the routing error is retained");
        assert!(source.to_string().contains("read the connection"));

        let failure = AdapterDeliveryFailure::from(AdapterError::from(
            SharedAdapterError::Internal("sandbox refused".into()),
        ));
        assert!(matches!(failure, AdapterDeliveryFailure::Retryable(_)));
    }

    #[test]
    fn a_failure_that_describes_the_event_is_terminal() {
        // `route_inbound_message` returns this both for a deactivated Loop
        // and for a subject with nothing left to deliver to. Neither improves
        // by being tried again.
        let failure = AdapterDeliveryFailure::from(AdapterError::from(
            SharedAdapterError::ConfigNotFound("tt_1".into()),
        ));
        assert!(matches!(failure, AdapterDeliveryFailure::Permanent(_)));
        let source = std::error::Error::source(&failure).expect("the routing error is retained");
        assert!(source.to_string().contains("tt_1"));
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions; the lints only fire when the db-tests feature is enabled"
)]
mod db_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use sqlx::PgPool;
    use tokio_util::sync::CancellationToken;

    use crate::adapters::delivery::{
        AdapterDeliveryPolicy, AdapterDeliveryWorker, dao::AdapterDeliveryStore,
    };
    use moatless_adapter_model::delivery::{
        AdapterDeliveryHandlers, AdapterDeliveryRequest, DeliveryAdmission,
    };

    use super::*;
    use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

    const KIND: &str = "test.inbound_message";

    /// Fails the way `route_inbound_message` fails when the backend, not the
    /// event, is at fault, classified through the same `From<AdapterError>` the
    /// real handler uses.
    ///
    /// The point is that this classification is what the shared worker policy
    /// retries.
    struct FailingRoute {
        runs: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl AdapterDeliveryHandler for FailingRoute {
        async fn execute(&self, _delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Err(AdapterDeliveryFailure::from(AdapterError::from(
                SharedAdapterError::Internal(
                    "Failed to create task: sandbox provisioning timed out".into(),
                ),
            )))
        }
    }

    /// Verification: an injected route failure is retried. The record spends
    /// its budget one attempt at a time, keeps its cause, and only then goes
    /// terminal — instead of the spawned path's single silent `warn!`.
    #[sqlx::test]
    async fn an_injected_route_failure_is_retried_then_recorded(pool: PgPool) {
        let store = Arc::new(AdapterDeliveryStore::with_policy(
            pool,
            AdapterDeliveryPolicy {
                max_attempts: 3,
                base_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(1),
                poll_interval: Duration::from_millis(5),
                maintenance_interval: Duration::from_millis(5),
                ..AdapterDeliveryPolicy::default()
            },
        ));
        store
            .admit(AdapterDeliveryRequest {
                adapter: "slack",
                kind: KIND,
                event_id: "Ev-route-fail".to_string(),
                payload: serde_json::json!({}),
            })
            .await
            .unwrap();

        let runs = Arc::new(AtomicUsize::new(0));
        let mut handlers = AdapterDeliveryHandlers::new();
        handlers.register(
            KIND,
            Arc::new(FailingRoute {
                runs: Arc::clone(&runs),
            }),
        );

        let cancel = CancellationToken::new();
        let handle = AdapterDeliveryWorker::new(Arc::clone(&store), "test-worker".to_string())
            .spawn(Arc::new(handlers), cancel.clone());
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
