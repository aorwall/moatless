use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::warn;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::ingress::{IngressError, Outcome};
use moatless_adapter_model::metrics::record_webhook;

use super::adapter::GithubPrAdapter;
use super::ingress::WebhookRequest;

#[utoipa::path(
    post,
    path = "/api/v1/adapters/github/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn github_webhook(
    State(github_pr_provider): State<Option<Arc<GithubPrAdapter>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AdapterError> {
    let github_pr_provider = github_pr_provider
        .ok_or_else(|| SharedAdapterError::AdapterNotConfigured("github_pr".into()))?;

    let event_type = header_value(&headers, "x-github-event");
    let request = WebhookRequest::new(
        event_type,
        header_value(&headers, "x-github-delivery"),
        header_value(&headers, "x-hub-signature-256"),
        &body,
    );

    // Every port `receive` needs is carried on the adapter (see
    // `with_ingress_ports`), so this route reads them through it rather than
    // an `AppState`. `receive` keeps its explicit-port contract; this route
    // just sources those ports from the provider it already extracts.
    match super::ingress::receive(
        github_pr_provider.require_adapter_connections()?,
        github_pr_provider.require_loops()?,
        github_pr_provider.require_users()?,
        github_pr_provider.require_task_bindings()?,
        github_pr_provider.require_adapter_delivery()?,
        &github_pr_provider,
        request,
    )
    .await
    {
        Ok(Outcome::Processed { event_type }) => {
            record_webhook("github_pr", "success", "", &event_type, "", "");
        }
        Ok(Outcome::Ignored {
            event_type,
            reason,
            subscription_id,
            connection_id,
        }) => {
            record_webhook(
                "github_pr",
                "ignored",
                reason,
                &event_type,
                &subscription_id,
                &connection_id,
            );
        }
        // The one failure that must not be acknowledged: nothing was written,
        // so a 200 would claim ownership of work with no record anywhere.
        // Surfaced as `503` at capacity, `500` on a failed write; GitHub won't
        // retry on its own, so this only leaves the delivery recorded as
        // failed and alertable via `not_admitted`.
        Err(IngressError::NotAdmitted(e)) => {
            record_webhook("github_pr", "error", "not_admitted", event_type, "", "");
            warn!(error = %e, "GitHub PR webhook work was not admitted; delivery left unacknowledged");
            return Err(AdapterError::from(e));
        }
        // Not constructed by `receive` today: unidentifiable GitHub deliveries
        // (no `repository.full_name`, no `X-GitHub-Event`) are raised as
        // `Ingress` and acknowledged. This arm exists so a step that starts
        // raising `Unidentifiable` gets a `400` instead of the metered `200`
        // below.
        Err(IngressError::Unidentifiable(missing)) => {
            record_webhook("github_pr", "error", "unidentifiable", event_type, "", "");
            warn!(%missing, "GitHub PR webhook could not be identified");
            return Err(SharedAdapterError::BadRequest(missing).into());
        }
        // Every other failure: log it, meter it, acknowledge. These reject the
        // delivery itself (bad header, bad JSON, bad signature), which a
        // redelivery cannot fix.
        Err(IngressError::Ingress(e)) => {
            record_webhook("github_pr", "error", "delivery_failed", event_type, "", "");
            warn!(error = %e, "GitHub PR webhook processing error");
        }
    }

    Ok(StatusCode::OK)
}

/// The event type the request advertised, for metric labelling when processing
/// failed before one was resolved.
fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}
