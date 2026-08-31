//! The webhook route. Transport only: read the request, hand the body and the
//! secret header to [`super::ingress::receive`], turn the outcome into a status.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::{info, warn};

use moatless_backend_core::identity::ProviderIdentityLink;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::ingress::{IngressError, Outcome};
use moatless_adapter_model::metrics::record_webhook;

use super::adapter::TelegramAdapter;
use super::ingress::{self, Transport};

#[utoipa::path(
    post,
    path = "/api/v1/adapters/telegram/webhook",
    tag = "adapters",
    request_body(content = Object),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn telegram_webhook(
    State(telegram_provider): State<Option<Arc<TelegramAdapter>>>,
    State(users): State<Arc<dyn ProviderIdentityLink>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AdapterError> {
    let telegram_provider = telegram_provider
        .ok_or_else(|| SharedAdapterError::AdapterNotConfigured("telegram".into()))?;

    info!("Telegram webhook received");

    let transport = Transport::Webhook {
        secret_header: headers
            .get("x-telegram-bot-api-secret-token")
            .and_then(|v| v.to_str().ok()),
    };

    match ingress::receive(&telegram_provider, users.as_ref(), &body, transport).await {
        // Routing is durably owned now; its outcome is recorded by the work
        // handler when it actually runs. Recording "success" here would count
        // an outcome this path no longer knows.
        Ok(Outcome::Processed { .. }) => {}
        Ok(Outcome::Ignored { reason, .. }) => {
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
            return Err(refusal(e));
        }
    }

    Ok(StatusCode::OK)
}

/// How each ingress class is refused. Every one of them is a non-2xx: Telegram
/// re-sends what it was not told we own.
fn refusal(error: IngressError) -> AdapterError {
    match error {
        IngressError::NotAdmitted(e) => AdapterError::from(e),
        IngressError::Unidentifiable(message) => SharedAdapterError::BadRequest(message).into(),
        IngressError::Ingress(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use moatless_adapter_model::delivery::AdmitError;
    use moatless_adapter_model::error::SharedAdapterError;
    use moatless_adapter_model::ingress::IngressError;
    use moatless_adapter_model::test_support::response_snapshot;
    use moatless_types::store::StoreError;

    use super::refusal;

    /// The acknowledgment class of each failure this route raises.
    ///
    /// The two `NotAdmitted` rows are the ones this Adapter could not reach
    /// before the shared vocabulary landed: `receive` admitted through a helper
    /// that flattened `AdmitError` into `AdapterError` first, so a refused
    /// admission arrived as an ingress rejection. It now admits through
    /// `moatless_loops::adapters::ingress::admit_delivery` — see the test beside that
    /// function — and this pins that the class it produces is still refused
    /// rather than acknowledged, and with the same statuses the flattened one
    /// produced.
    #[tokio::test]
    async fn no_failure_on_the_telegram_webhook_path_is_acknowledged() {
        for (error, expected) in [
            (
                IngressError::Ingress(SharedAdapterError::SignatureInvalid.into()),
                StatusCode::UNAUTHORIZED,
            ),
            (
                IngressError::Unidentifiable("Missing update_id".into()),
                StatusCode::BAD_REQUEST,
            ),
            (
                IngressError::NotAdmitted(AdmitError::AtCapacity { limit: 10 }),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                IngressError::NotAdmitted(AdmitError::Store(StoreError::new(
                    "persist the Adapter delivery record",
                ))),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            let (status, _) = response_snapshot(refusal(error).into_response()).await;
            assert_eq!(status, expected);
            assert_ne!(status, StatusCode::OK, "a failure must not acknowledge");
        }
    }
}
