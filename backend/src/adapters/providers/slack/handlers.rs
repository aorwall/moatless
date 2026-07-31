use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tracing::debug;

use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::state::AppState;

use super::ingress::{IngressError, Outcome, WebhookRequest};

#[utoipa::path(
    post,
    path = "/api/v1/adapters/slack/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn slack_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AdapterError> {
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("slack".into()))?;

    let slack_provider = providers
        .slack()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("slack".into()))?;

    let request = WebhookRequest {
        timestamp: header_value(&headers, "x-slack-request-timestamp"),
        signature: header_value(&headers, "x-slack-signature"),
        body: &body,
    };

    match super::ingress::receive(&state, slack_provider, request).await {
        // Slack's endpoint check wants the challenge echoed in the body.
        Ok(Outcome::Challenge(challenge)) => Ok((StatusCode::OK, challenge).into_response()),
        // No metric: the delivery worker records the outcome of the work that
        // was just admitted, and recording one here would double-count it.
        Ok(Outcome::Processed { event_type }) => {
            debug!(event_type, "Slack webhook admitted for routing");
            Ok(StatusCode::OK.into_response())
        }
        Ok(Outcome::Ignored {
            event_type,
            reason,
            rule_id,
            connection_id,
        }) => {
            record_webhook(
                "slack",
                "ignored",
                reason,
                &event_type,
                &rule_id,
                &connection_id,
            );
            Ok(StatusCode::OK.into_response())
        }
        // Nothing durable was written, so a 200 would tell Slack the work is
        // ours while no record of it exists anywhere. Surfaced as `503` at
        // capacity and `500` on a failed write; Slack re-delivers either way.
        Err(IngressError::NotAdmitted(e)) => Err(AdapterError::from(e)),
        // No retry supplies what the request was missing.
        Err(IngressError::Unidentifiable(message)) => Err(AdapterError::BadRequest(message)),
        // A rejection of the delivery itself — a failed signature, a failed
        // read. Answered as the failure it is, exactly as before.
        Err(IngressError::Ingress(e)) => Err(e),
    }
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use crate::adapters::delivery::AdmitError;
    use crate::adapters::error::AdapterError;
    use crate::adapters::test_support::response_snapshot;

    use super::super::ingress::IngressError;

    /// The acknowledgment class of each failure the Slack webhook path raises.
    /// An ingress rejection and an admission failure both leave the request
    /// unacknowledged — Slack re-delivers, and nothing durable was lost.
    ///
    /// Every row is reachable: `receive` admits through `admit_delivery`, which
    /// hands back `AdmitError` unflattened, so both `NotAdmitted` rows are
    /// states a real at-capacity or failed write produces — see
    /// [`IngressError::NotAdmitted`] for why that is not true of every Adapter.
    #[tokio::test]
    async fn no_failure_on_the_slack_webhook_path_is_acknowledged() {
        for (error, expected) in [
            (
                IngressError::Ingress(AdapterError::SignatureInvalid),
                StatusCode::UNAUTHORIZED,
            ),
            (
                IngressError::Unidentifiable("Invalid JSON: x".into()),
                StatusCode::BAD_REQUEST,
            ),
            (
                IngressError::NotAdmitted(AdmitError::AtCapacity { limit: 10 }),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                IngressError::NotAdmitted(AdmitError::Store(sqlx::Error::PoolTimedOut)),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            // The mapping the handler applies, spelled out on its own: the
            // handler itself needs an `AppState` to reach.
            let mapped = match error {
                IngressError::NotAdmitted(e) => AdapterError::from(e),
                IngressError::Unidentifiable(message) => AdapterError::BadRequest(message),
                IngressError::Ingress(e) => e,
            };
            let (status, _) = response_snapshot(mapped.into_response()).await;
            assert_eq!(status, expected);
            assert_ne!(status, StatusCode::OK, "a failure must not acknowledge");
        }
    }
}
