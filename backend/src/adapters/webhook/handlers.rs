use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use tracing::{info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/adapters/webhook/{trigger_id}",
    tag = "adapters",
    request_body = super::adapter::WebhookPayload,
    responses((status = 200, description = "Webhook accepted"))
)]
pub async fn webhook_inbound(
    State(state): State<AppState>,
    Path(trigger_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, AdapterError> {
    use axum::response::IntoResponse;

    info!(trigger_id = %trigger_id, "Webhook inbound received");

    let rule = state.loops
        .find_subscription_by_id(&trigger_id)
        .await?
        .ok_or_else(|| AdapterError::ConfigNotFound(trigger_id.clone()))?;

    if rule.provider != "webhook" {
        return Err(AdapterError::BadRequest(format!(
            "Trigger {trigger_id} is not a webhook trigger"
        )));
    }
    if !rule.active {
        record_webhook(
            "webhook",
            "ignored",
            "inactive",
            "",
            &trigger_id,
            &rule.connection_id,
        );
        return Ok(StatusCode::OK.into_response());
    }

    let connection = PgAdapterConnectionRepository(state.db.clone())
        .find_by_id(&rule.connection_id)
        .await?
        .ok_or_else(|| {
            AdapterError::NotFound(format!("Connection {} not found", rule.connection_id))
        })?;

    let signature = headers
        .get("x-webhook-signature")
        .and_then(|v| v.to_str().ok());
    let config = crate::bindings::runtime::trigger_context(&rule, "webhook");
    let msg = super::adapter::WebhookAdapter::parse_inbound(&config, &body, signature)?;
    let thread_id = msg.thread_id.as_deref().unwrap_or(&msg.message_thread_id);
    let subject_key = format!("{}:{thread_id}", msg.source_id);

    match crate::bindings::runtime::route_inbound_message(
        &state,
        &connection,
        &rule,
        msg,
        "webhook",
        "webhook_thread",
        &subject_key,
        serde_json::json!({}),
    )
    .await
    {
        Ok(task_id) => {
            record_webhook("webhook", "success", "", "", &trigger_id, &connection.id);
            info!(task_id = %task_id, trigger_id = %trigger_id, "Webhook routed to task");
            Ok(Json(serde_json::json!({ "taskId": task_id })).into_response())
        }
        Err(e) => {
            record_webhook(
                "webhook",
                "error",
                "delivery_failed",
                "",
                &trigger_id,
                &connection.id,
            );
            warn!(trigger_id = %trigger_id, error = %e, "Webhook routing failed");
            Err(e)
        }
    }
}
