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
    path = "/api/v1/adapters/github/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AdapterError> {
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("github_pr".into()))?;

    let github_pr = providers
        .get(AdapterKind::GithubPr)
        .ok_or_else(|| AdapterError::AdapterNotConfigured("github_pr".into()))?;

    let github_pr_provider = github_pr
        .as_any()
        .downcast_ref::<super::adapter::GithubPrAdapter>()
        .ok_or_else(|| AdapterError::Internal("Provider type mismatch".into()))?;

    match super::adapter::resolve_and_process_github_webhook(
        &state,
        github_pr_provider,
        &headers,
        &body,
    )
    .await
    {
        Ok(()) => {
            record_webhook("github_pr", "success", "");
        }
        Err(e) => {
            record_webhook("github_pr", "error", "delivery_failed");
            warn!(error = %e, "GitHub PR webhook processing error");
        }
    }

    Ok(StatusCode::OK)
}
