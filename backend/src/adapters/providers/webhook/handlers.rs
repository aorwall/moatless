use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use tracing::{debug, info};

use crate::adapters::delivery::AdapterDeliveryStore;
use crate::adapters::error::AdapterError;
use crate::adapters::inbound_delivery::{inbound_delivery_request, InboundDeliveryPayload};
use crate::adapters::ingress::admit_delivery;
use crate::loops::dao::LoopDao;

use super::adapter::{WebhookAccepted, WebhookAdapter};
use super::delivery::{ADAPTER, INBOUND_MESSAGE, SUBJECT_KIND};

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
    State(loops): State<Arc<LoopDao>>,
    State(adapter_delivery): State<Arc<AdapterDeliveryStore>>,
    Path(loop_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WebhookAccepted>, AdapterError> {
    info!(loop_id = %loop_id, "Webhook inbound received");

    let subscription = loops
        .find_runnable_subscription_by_id(&loop_id)
        .await?
        .ok_or_else(|| AdapterError::ConfigNotFound(loop_id.clone()))?;

    if subscription.adapter_kind != ADAPTER {
        return Err(AdapterError::BadRequest(format!(
            "Loop {loop_id} is not a webhook Subscription"
        )));
    }

    let signature = headers
        .get("x-webhook-signature")
        .and_then(|v| v.to_str().ok());
    let config = moatless_adapter_model::models::subscription_context(&subscription, ADAPTER);
    let delivery = WebhookAdapter::parse_inbound(&config, &body, signature)?;
    let message = delivery.message;
    let thread_id = message
        .thread_id
        .as_deref()
        .unwrap_or(&message.message_thread_id);
    let subject_key = format!("{}:{thread_id}", message.source_id);

    // The Adapter connection is named, not read. Admission runs on the caller's
    // response deadline and must not fail because a connection is missing or
    // concurrently deleted; the handler reads it when it executes and reports a
    // permanent failure if it is gone.
    //
    // Everything below the 200 depends on this write: a failure here must reach
    // the caller as a failure so it re-delivers, and must never be swallowed
    // into an acknowledgment. The routing itself now runs in the Adapter
    // delivery worker, which is also where the webhook outcome metric is
    // recorded.
    let request = inbound_delivery_request(
        ADAPTER,
        INBOUND_MESSAGE,
        delivery.delivery_id.clone(),
        InboundDeliveryPayload {
            connection_id: subscription.connection_id.clone(),
            loop_id: subscription.id.clone(),
            subject_kind: SUBJECT_KIND.to_string(),
            subject_key,
            destination_config: serde_json::json!({}),
            event_type: String::new(),
            message,
        },
    )?;
    // This handler has no `receive` to hold the shared `Outcome`/`IngressError`
    // — it answers with `AdapterError` directly and its own body type — so the
    // `AdmitError` is flattened here, by the same `From` impl that used to do it
    // inside the admitting helper. Giving it the shared vocabulary means giving
    // it an ingress module, which is stage 10's job, not this one's.
    let admission = admit_delivery(&adapter_delivery, request).await?;

    debug!(
        loop_id = %loop_id,
        delivery_id = %delivery.delivery_id,
        admission = admission.as_str(),
        "Webhook delivery admitted for routing"
    );
    Ok(Json(WebhookAccepted {
        delivery_id: delivery.delivery_id,
        status: admission.into(),
    }))
}
