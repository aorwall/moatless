use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::warn;

use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::adapters::AdapterKind;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/adapters/linear/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn linear_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AdapterError> {
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;

    let linear = providers
        .get(AdapterKind::Linear)
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;

    let linear_provider = linear
        .as_any()
        .downcast_ref::<super::adapter::LinearAdapter>()
        .ok_or_else(|| AdapterError::Internal("Provider type mismatch".into()))?;

    match super::adapter::resolve_and_process_linear_webhook(
        &state,
        linear_provider,
        &headers,
        &body,
    )
    .await
    {
        Ok(()) => {
            record_webhook("linear", "success", "", "", "");
        }
        Err(e) => {
            record_webhook("linear", "error", "delivery_failed", "", "");
            warn!(error = %e, "Linear webhook processing error");
        }
    }

    Ok(StatusCode::OK)
}
