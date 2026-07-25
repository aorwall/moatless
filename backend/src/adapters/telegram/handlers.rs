use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::{info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/adapters/telegram/webhook",
    tag = "adapters",
    request_body(content = Object),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn telegram_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, AdapterError> {
    use axum::response::IntoResponse;

    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("telegram".into()))?;

    let telegram_provider = providers
        .telegram()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("telegram".into()))?;

    info!("Telegram webhook received");

    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AdapterError::BadRequest(format!("Invalid JSON: {e}")))?;

    let message = payload
        .get("message")
        .or_else(|| payload.get("channel_post"));
    if let Some(chat_id) = message
        .and_then(|m| m.pointer("/chat/id"))
        .and_then(serde_json::Value::as_i64)
    {
        let source_id = chat_id.to_string();
        let chat_type = message
            .and_then(|m| m.pointer("/chat/type"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let is_private_chat = chat_type == "private";

        let rule = crate::bindings::runtime::resolve_chat_provider_event(
            &state,
            "telegram",
            "default",
            if is_private_chat { "*" } else { &source_id },
        )
        .await?
        .map(|(_, rule)| rule);

        if let Some(ref cfg) = rule {
            let expected_secret = cfg
                .provider_config
                .as_ref()
                .and_then(|c| c.get("secret_token"))
                .and_then(|v| v.as_str());

            if let Some(expected) = expected_secret {
                let provided = headers
                    .get("x-telegram-bot-api-secret-token")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");

                if provided != expected {
                    return Err(AdapterError::SignatureInvalid);
                }
            }
        }
    }

    match super::adapter::resolve_and_process_telegram_update(&state, telegram_provider, &body)
        .await
    {
        Ok(()) => {
            record_webhook("telegram", "success", "", "", "", "");
        }
        Err(e) => {
            record_webhook("telegram", "error", "delivery_failed", "", "", "");
            warn!(error = %e, "Telegram webhook processing error");
        }
    }

    Ok(StatusCode::OK.into_response())
}
