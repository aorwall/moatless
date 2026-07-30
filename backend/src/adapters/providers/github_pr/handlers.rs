use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::warn;

use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::state::AppState;

use super::adapter::GithubWebhookError;

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
        // The one failure that must not be acknowledged. Nothing was written,
        // so a 200 would tell GitHub the work is ours while no record of it
        // exists anywhere. Surfaced as `503` at capacity and `500` on a failed
        // write. GitHub does not retry a delivery on its own, so what this buys
        // is a delivery recorded as failed — redeliverable by an operator for 30
        // days, and alertable here via the `not_admitted` reason.
        Err(GithubWebhookError::NotAdmitted(e)) => {
            let event_type = advertised_event_type(&headers);
            record_webhook("github_pr", "error", "not_admitted", event_type, "", "");
            warn!(error = %e, "GitHub PR webhook work was not admitted; delivery left unacknowledged");
            return Err(AdapterError::from(e));
        }
        // Every other failure keeps the long-standing behaviour: log it, meter
        // it, and acknowledge. These are rejections of the delivery itself
        // (bad header, bad JSON, bad signature), which a redelivery cannot fix.
        Err(GithubWebhookError::Ingress(e)) => {
            let event_type = advertised_event_type(&headers);
            record_webhook("github_pr", "error", "delivery_failed", event_type, "", "");
            warn!(error = %e, "GitHub PR webhook processing error");
        }
    }

    Ok(StatusCode::OK)
}

/// The event type the request advertised, for metric labelling when processing
/// failed before one was resolved.
fn advertised_event_type(headers: &HeaderMap) -> &str {
    headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}
