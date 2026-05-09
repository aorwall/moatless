use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::warn;

use crate::channel::error::ChannelError;
use crate::channel::provider::ChannelType;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/integrations/linear/webhook",
    tag = "integrations",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn linear_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ChannelError> {
    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("linear".into()))?;

    let linear = providers
        .get(ChannelType::Linear)
        .ok_or_else(|| ChannelError::ProviderNotConfigured("linear".into()))?;

    let linear_provider = linear
        .as_any()
        .downcast_ref::<super::provider::LinearProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    if let Err(e) = super::provider::resolve_and_process_linear_webhook(
        &state,
        linear_provider,
        &headers,
        &body,
    )
    .await
    {
        warn!(error = %e, "Linear webhook processing error");
    }

    Ok(StatusCode::OK)
}
