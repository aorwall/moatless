use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;

use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::loops::LoopReader;

use moatless_api_schemas::webhook::WebhookAccepted;

/// Take durable ownership of one inbound webhook delivery.
///
/// Everything the caller is answered with is decided before any routing runs:
/// the delivery is verified, parsed, and written to the Adapter delivery seam,
/// and only then acknowledged. A 200 therefore means the backend owns the delivery,
/// not that a Task exists — the Task is created later, by the worker, out of the
/// record this wrote. Anything that failed to record reaches the caller as a
/// failure so its own retry is the recovery path.
///
/// Deactivated loops are rejected before admission. A Loop can also be
/// paused or resynced after admission; the worker rechecks runnable state and
/// then terminates the durable delivery without creating a Task.
#[utoipa::path(
    post,
    path = "/api/v1/adapters/webhook/{loop_id}",
    tag = "adapters",
    request_body = super::adapter::WebhookPayload,
    responses((
        status = 200,
        description = "The delivery is owned by the backend. Names the delivery, not a Task.",
        body = WebhookAccepted
    ))
)]
pub async fn webhook_inbound(
    State(loops): State<Arc<dyn LoopReader>>,
    State(adapter_delivery): State<Arc<dyn DeliveryAdmission>>,
    Path(loop_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WebhookAccepted>, AdapterError> {
    let signature = headers
        .get("x-webhook-signature")
        .and_then(|value| value.to_str().ok());
    crate::ingress::receive(
        loops.as_ref(),
        adapter_delivery.as_ref(),
        loop_id,
        signature,
        &body,
    )
    .await
    .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::INBOUND_MESSAGE;
    use crate::adapter::WebhookAdmission;

    use async_trait::async_trait;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use moatless_adapter_model::test_support::{
        response_snapshot, FakeAdmission, FakeAdmissionAnswer,
    };
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::AdapterKind;
    use moatless_types::loops::RoutingMode;
    use moatless_types::loops::SubscriptionMatch;
    use moatless_types::store::StoreError;

    /// A runnable webhook Subscription, the one shape every test here drives the
    /// handler with. No `webhook_secret`, so admission is reached without a
    /// signature.
    fn webhook_rule() -> SubscriptionMatch {
        let now = chrono::Utc::now();
        SubscriptionMatch {
            id: "tt_1".into(),
            name: "test-loop".into(),
            adapter_kind: AdapterKind::Webhook,
            connection_id: "pc_test".into(),
            source_matcher: "my-webhook".into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repo-1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            skills: vec![],
            run_as_user_id: Some("u_test".into()),
            agent_type: "claude-code".into(),
            model: None,
            inactive_ttl_days: None,
            allow_direct_send: true,
            adapter_config: None,
            ongoing_task_id: None,
            close_task_on_close: true,
            notify_on_success: false,
            active: true,
            created_by: "system".into(),
            scope: Scope::Global,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// A `LoopReader` that answers every read with the one runnable webhook
    /// Subscription. The handler only calls `runnable_subscription`.
    struct RunnableLoop;

    #[async_trait]
    impl LoopReader for RunnableLoop {
        async fn subscription(&self, _: &str) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(Some(webhook_rule()))
        }
        async fn runnable_subscription(
            &self,
            _: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(Some(webhook_rule()))
        }
        async fn subscription_for_source(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(None)
        }
        async fn subscriptions_for_source(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<SubscriptionMatch>, StoreError> {
            Ok(vec![])
        }
        async fn wildcard_subscription(
            &self,
            _: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(None)
        }
    }

    fn runnable_loop() -> Arc<dyn LoopReader> {
        Arc::new(RunnableLoop)
    }

    fn body() -> Bytes {
        Bytes::from_static(br#"{"text":"hello"}"#)
    }

    fn body_with_delivery_id(id: &str) -> Bytes {
        Bytes::from(format!(r#"{{"text":"hello","deliveryId":"{id}"}}"#))
    }

    /// A refused admission must reach the caller as a failure, or the caller
    /// stops re-delivering work nothing durable was written for.
    #[tokio::test]
    async fn a_full_backlog_is_not_acknowledged() {
        let fake = Arc::new(FakeAdmission::new(FakeAdmissionAnswer::AtCapacity {
            limit: 10,
        }));
        let admission: Arc<dyn DeliveryAdmission> = fake.clone();
        let refused = webhook_inbound(
            State(runnable_loop()),
            State(admission),
            Path("tt_1".into()),
            HeaderMap::new(),
            body(),
        )
        .await;

        let status = refused
            .expect_err("at capacity is a refusal")
            .into_response()
            .status();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(fake.seen().len(), 1, "the request reached the seam");
    }

    /// A redelivery is answered 200 with nothing written — the property the
    /// unique index buys, asserted without a database.
    #[tokio::test]
    async fn a_redelivery_is_acknowledged_and_writes_nothing() {
        let admission: Arc<dyn DeliveryAdmission> =
            Arc::new(FakeAdmission::new(FakeAdmissionAnswer::Duplicate));
        let Json(accepted) = webhook_inbound(
            State(runnable_loop()),
            State(admission),
            Path("tt_1".into()),
            HeaderMap::new(),
            body(),
        )
        .await
        .expect("a duplicate is still owned");

        assert_eq!(accepted.status, WebhookAdmission::Duplicate);
    }

    /// The committed case, and the only place the sender's own delivery id is
    /// asserted to survive into the durable key.
    #[tokio::test]
    async fn an_admitted_delivery_carries_the_senders_own_id() {
        let fake = Arc::new(FakeAdmission::new(FakeAdmissionAnswer::Admitted));
        let admission: Arc<dyn DeliveryAdmission> = fake.clone();
        let Json(accepted) = webhook_inbound(
            State(runnable_loop()),
            State(admission),
            Path("tt_1".into()),
            HeaderMap::new(),
            body_with_delivery_id("d-7"),
        )
        .await
        .expect("a committed record is owned");

        assert_eq!(accepted.status, WebhookAdmission::Admitted);
        let seen = fake.seen();
        assert_eq!(seen[0].event_id, "d-7");
        assert_eq!(seen[0].kind, INBOUND_MESSAGE);
    }

    /// The redaction boundary: the cause is kept as a source, never in the body.
    #[tokio::test]
    async fn a_failed_write_is_a_redacted_500() {
        let admission: Arc<dyn DeliveryAdmission> =
            Arc::new(FakeAdmission::new(FakeAdmissionAnswer::StoreFailed));
        let failed = webhook_inbound(
            State(runnable_loop()),
            State(admission),
            Path("tt_1".into()),
            HeaderMap::new(),
            body(),
        )
        .await
        .expect_err("a failed write is not an acknowledgment");

        let (status, body) = response_snapshot(failed.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }
}
