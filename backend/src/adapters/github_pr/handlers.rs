use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::warn;

use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
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

    let github_pr_provider = providers
        .github_pr()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("github_pr".into()))?;

    match super::adapter::resolve_and_process_github_webhook(
        &state,
        github_pr_provider,
        &headers,
        &body,
    )
    .await
    {
        Ok(super::adapter::GithubWebhookOutcome::Processed { event_type }) => {
            record_webhook("github_pr", "success", "", &event_type, "", "");
        }
        Ok(super::adapter::GithubWebhookOutcome::Ignored { event_type, reason }) => {
            record_webhook("github_pr", "ignored", reason, &event_type, "", "");
        }
        Err(e) => {
            // `event_type` may have been unset (e.g. missing header); fall back
            // to whatever the request advertised so the metric still has a
            // useful label.
            let event_type = headers
                .get("x-github-event")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            record_webhook("github_pr", "error", "delivery_failed", event_type, "", "");
            warn!(error = %e, "GitHub PR webhook processing error");
        }
    }

    Ok(StatusCode::OK)
}
