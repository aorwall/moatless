use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::{info, warn};

use crate::channel::error::ChannelError;
use crate::channel::provider::ChannelType;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/integrations/telegram/webhook",
    tag = "integrations",
    request_body(content = Object),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn telegram_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, ChannelError> {
    use axum::response::IntoResponse;

    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("telegram".into()))?;

    let telegram = providers
        .get(ChannelType::Telegram)
        .ok_or_else(|| ChannelError::ProviderNotConfigured("telegram".into()))?;

    let telegram_provider = telegram
        .as_any()
        .downcast_ref::<super::provider::TelegramProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    info!("Telegram webhook received");

    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

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

        let rule = crate::channel::binding_runtime::resolve_chat_provider_event(
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
                    return Err(ChannelError::SignatureInvalid);
                }
            }
        }
    }

    if let Err(e) =
        super::provider::resolve_and_process_telegram_update(&state, telegram_provider, &body).await
    {
        warn!(error = %e, "Telegram webhook processing error");
    }

    Ok(StatusCode::OK.into_response())
}
