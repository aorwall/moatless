use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::warn;

use crate::channel::error::ChannelError;
use crate::channel::provider::ChannelType;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/integrations/github/webhook",
    tag = "integrations",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ChannelError> {
    let providers = state
        .channel_providers
        .as_ref()
        .ok_or_else(|| ChannelError::ProviderNotConfigured("github_pr".into()))?;

    let github_pr = providers
        .get(ChannelType::GithubPr)
        .ok_or_else(|| ChannelError::ProviderNotConfigured("github_pr".into()))?;

    let github_pr_provider = github_pr
        .as_any()
        .downcast_ref::<super::provider::GithubPrProvider>()
        .ok_or_else(|| ChannelError::Internal("Provider type mismatch".into()))?;

    if let Err(e) = super::provider::resolve_and_process_github_webhook(
        &state,
        github_pr_provider,
        &headers,
        &body,
    )
    .await
    {
        warn!(error = %e, "GitHub PR webhook processing error");
    }

    Ok(StatusCode::OK)
}
