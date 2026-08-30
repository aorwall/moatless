//! Linear webhook endpoint.
//!
//! Transport only: pull the signature and the body off the request, hand them
//! to [`ingress::receive`](super::ingress::receive), and turn what comes back
//! into a status code and one metric row. Every decision about what a delivery
//! means — including which of them are refused, and why — lives below this
//! file, which is what lets a test drive the whole path without building a
//! `Request`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::warn;

use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::metrics::record_webhook;

use super::adapter::LinearAdapter;
use super::ingress::{IngressError, Outcome};

#[utoipa::path(
    post,
    path = "/api/v1/adapters/linear/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn linear_webhook(
    State(linear_provider): State<Option<Arc<LinearAdapter>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AdapterError> {
    let linear_provider =
        linear_provider.ok_or_else(|| SharedAdapterError::AdapterNotConfigured("linear".into()))?;

    let signature = headers
        .get("linear-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    match super::ingress::receive(
        linear_provider.require_adapter_connections()?,
        linear_provider.require_loops()?,
        linear_provider.app_tokens(),
        linear_provider.require_apps()?,
        linear_provider.require_adapter_delivery()?,
        &linear_provider,
        signature,
        &body,
    )
    .await
    {
        Ok(Outcome::Admitted { event_type, .. }) => {
            record_webhook("linear", "success", "", &event_type, "", "");
        }
        // Arrived and dropped, which until this shipped was indistinguishable
        // from arrived and admitted — every one of these paths metered
        // `success` (defect D1). The reason list is a contract; see
        // `ingress::reason`.
        Ok(Outcome::Ignored { event_type, reason }) => {
            record_webhook("linear", "ignored", reason, &event_type, "", "");
        }
        // Nothing was written, so a 200 would tell Linear the delivery is ours
        // while no record of it exists — and Linear does not redeliver an
        // acknowledged event.
        Err(rejection) => {
            let event_type = rejection.event_type;
            match rejection.error {
                IngressError::NotAdmitted(e) => {
                    record_webhook("linear", "error", "not_admitted", &event_type, "", "");
                    warn!(error = %e, "Linear webhook work was not admitted; asking Linear to redeliver");
                    return Err(AdapterError::from(e));
                }
                // Also not acknowledged, but for the opposite reason: admission
                // was never attempted, because the delivery carries no field
                // that could tell its work apart from the next delivery's. A
                // `400` says so — the delivery is malformed rather than the
                // backend unwell, and no amount of retrying the same body will
                // add the missing field.
                IngressError::Unidentifiable(missing) => {
                    record_webhook("linear", "error", "unidentifiable", &event_type, "", "");
                    warn!(
                        missing,
                        "Linear webhook refused: the delivery cannot be given an identity"
                    );
                    return Err(SharedAdapterError::BadRequest(format!(
                        "Linear delivery is missing {missing}"
                    ))
                    .into());
                }
                // Everything else keeps the long-standing behaviour: log it,
                // meter it, and acknowledge.
                IngressError::Ingress(e) => {
                    record_webhook("linear", "error", "delivery_failed", &event_type, "", "");
                    warn!(error = %e, "Linear webhook processing error");
                }
            }
        }
    }

    Ok(StatusCode::OK)
}
