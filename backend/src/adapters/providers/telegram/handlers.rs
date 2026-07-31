//! The webhook route. Transport only: read the request, hand the body and the
//! secret header to [`super::ingress::receive`], turn the outcome into a status.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::{info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::state::AppState;

use super::ingress::{self, IngressError, Outcome, Transport};

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
) -> Result<StatusCode, AdapterError> {
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("telegram".into()))?;

    let telegram_provider = providers
        .telegram()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("telegram".into()))?;

    info!("Telegram webhook received");

    let transport = Transport::Webhook {
        secret_header: headers
            .get("x-telegram-bot-api-secret-token")
            .and_then(|v| v.to_str().ok()),
    };

    match ingress::receive(&state, telegram_provider, &body, transport).await {
        // Routing is durably owned now; its outcome is recorded by the work
        // handler when it actually runs. Recording "success" here would count
        // an outcome this path no longer knows.
        Ok(Outcome::Admitted) => {}
        Ok(Outcome::Ignored { reason }) => {
            info!(reason, "Telegram update ignored");
            // The typed reason is logged, not metered: `not_routable` stays the
            // one label Telegram's dashboards query (spec Decision 9).
            record_webhook("telegram", "ignored", "not_routable", "", "", "");
        }
        Err(e) => {
            record_webhook("telegram", "error", "delivery_failed", "", "", "");
            warn!(error = %e, "Telegram webhook processing error");
            // Nothing durable was written, so this delivery is still Telegram's.
            // Acknowledging it here is what used to lose the update outright.
            return Err(match e {
                IngressError::NotAdmitted(e) => AdapterError::from(e),
                IngressError::Unidentifiable(message) => AdapterError::BadRequest(message),
                IngressError::Ingress(e) => e,
            });
        }
    }

    Ok(StatusCode::OK)
}
