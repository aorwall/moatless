use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use tracing::{info, warn};

use crate::channel::dao::connections::PgProviderConnectionRepository;
use crate::channel::error::ChannelError;
use crate::channel::services::provider_event_delivery::ProviderEventDeliveryService;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/integrations/webhook/{trigger_id}",
    tag = "integrations",
    request_body = super::provider::WebhookPayload,
    responses((status = 200, description = "Webhook accepted"))
)]
pub async fn webhook_inbound(
    State(state): State<AppState>,
    Path(trigger_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, ChannelError> {
    use axum::response::IntoResponse;

    info!(trigger_id = %trigger_id, "Webhook inbound received");

    let rule = crate::channel::dao::trigger_routes::PgTaskTriggerRouteRepository(state.db.clone())
        .find_by_id(&trigger_id)
        .await?
        .ok_or_else(|| ChannelError::ConfigNotFound(trigger_id.clone()))?;

    if rule.provider != "webhook" {
        return Err(ChannelError::BadRequest(format!(
            "Trigger {trigger_id} is not a webhook trigger"
        )));
    }
    if !rule.active {
        return Ok(StatusCode::OK.into_response());
    }

    let connection = PgProviderConnectionRepository(state.db.clone())
        .find_by_id(&rule.connection_id)
        .await?
        .ok_or_else(|| {
            ChannelError::NotFound(format!("Connection {} not found", rule.connection_id))
        })?;

    let signature = headers
        .get("x-webhook-signature")
        .and_then(|v| v.to_str().ok());
    let config = crate::channel::binding_runtime::trigger_context(&rule, "webhook");
    let msg = super::provider::WebhookProvider::parse_inbound(&config, &body, signature)?;
    let thread_id = msg.thread_id.as_deref().unwrap_or(&msg.message_thread_id);
    let subject_key = format!("{}:{thread_id}", msg.source_id);

    match ProviderEventDeliveryService::new(&state)
        .deliver(
            &connection,
            &rule,
            msg,
            None,
            "webhook",
            "webhook_thread",
            &subject_key,
            serde_json::json!({}),
        )
        .await
    {
        Ok(task_id) => {
            info!(task_id = %task_id, trigger_id = %trigger_id, "Webhook routed to task");
            Ok(Json(serde_json::json!({ "taskId": task_id })).into_response())
        }
        Err(e) => {
            warn!(trigger_id = %trigger_id, error = %e, "Webhook routing failed");
            Err(e)
        }
    }
}
