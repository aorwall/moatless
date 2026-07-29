use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use tracing::{debug, info};

use crate::adapters::error::AdapterError;
use crate::adapters::inbound_delivery::{admit_inbound_message, InboundDeliveryPayload};
use crate::state::AppState;

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
/// That includes a delivery to a deactivated trigger: it is admitted like any
/// other and ends terminally when the worker resolves the trigger and finds it
/// inactive. Answering it without a record would be an acknowledgment with
/// nothing behind it, which is the thing this route exists to stop.
#[utoipa::path(
    post,
    path = "/api/v1/adapters/webhook/{trigger_id}",
    tag = "adapters",
    request_body = super::adapter::WebhookPayload,
    responses((
        status = 200,
        description = "The delivery is owned by the backend. Names the delivery, not a Task.",
        body = WebhookAccepted
    ))
)]
pub async fn webhook_inbound(
    State(state): State<AppState>,
    Path(trigger_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WebhookAccepted>, AdapterError> {
    info!(trigger_id = %trigger_id, "Webhook inbound received");

    let rule = state
        .loops
        .find_subscription_by_id(&trigger_id)
        .await?
        .ok_or_else(|| AdapterError::ConfigNotFound(trigger_id.clone()))?;

    if rule.provider != ADAPTER {
        return Err(AdapterError::BadRequest(format!(
            "Trigger {trigger_id} is not a webhook trigger"
        )));
    }

    let signature = headers
        .get("x-webhook-signature")
        .and_then(|v| v.to_str().ok());
    let config = crate::bindings::runtime::trigger_context(&rule, ADAPTER);
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
    let admission = admit_inbound_message(
        &state,
        ADAPTER,
        INBOUND_MESSAGE,
        delivery.delivery_id.clone(),
        InboundDeliveryPayload {
            connection_id: rule.connection_id.clone(),
            rule_id: rule.id.clone(),
            subject_kind: SUBJECT_KIND.to_string(),
            subject_key,
            destination_config: serde_json::json!({}),
            event_type: String::new(),
            message,
        },
    )
    .await?;

    debug!(
        trigger_id = %trigger_id,
        delivery_id = %delivery.delivery_id,
        admission = admission.as_str(),
        "Webhook delivery admitted for routing"
    );
    Ok(Json(WebhookAccepted {
        delivery_id: delivery.delivery_id,
        status: admission.into(),
    }))
}
