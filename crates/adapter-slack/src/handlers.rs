use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tracing::debug;

use moatless_adapter_model::binding::BindingProbe;
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::ingress::{IngressError, Outcome};
use moatless_adapter_model::loops::LoopReader;
use moatless_adapter_model::metrics::record_webhook;
use moatless_ports::identity::ProviderIdentityLink;

use super::adapter::SlackAdapter;
use super::ingress::{Received, WebhookRequest};

#[utoipa::path(
    post,
    path = "/api/v1/adapters/slack/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
#[expect(
    clippy::too_many_arguments,
    reason = "each of the five downward ports the ingress reads (adapter \
              connections, loops, task bindings, identity links, adapter \
              delivery) is its \
              own `State<Port>` extractor via `FromRef<AppState>`, plus the \
              provider and the two transport facts; grouping the ports back \
              into one extractor would reintroduce the AppState-shaped bag this \
              slice removes"
)]
pub async fn slack_webhook(
    State(adapter_connections): State<Arc<dyn AdapterConnectionDao>>,
    State(loops): State<Arc<dyn LoopReader>>,
    State(task_bindings): State<Arc<dyn BindingProbe>>,
    State(users): State<Arc<dyn ProviderIdentityLink>>,
    State(adapter_delivery): State<Arc<dyn DeliveryAdmission>>,
    State(slack_provider): State<Option<Arc<SlackAdapter>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AdapterError> {
    let slack_provider =
        slack_provider.ok_or_else(|| SharedAdapterError::AdapterNotConfigured("slack".into()))?;

    let request = WebhookRequest {
        timestamp: header_value(&headers, "x-slack-request-timestamp"),
        signature: header_value(&headers, "x-slack-signature"),
        body: &body,
    };

    match super::ingress::receive(
        adapter_connections.as_ref(),
        loops.as_ref(),
        task_bindings.as_ref(),
        users.as_ref(),
        adapter_delivery.as_ref(),
        &slack_provider,
        request,
    )
    .await
    {
        // Slack's endpoint check wants the challenge echoed in the body.
        Ok(Received::Challenge(challenge)) => Ok((StatusCode::OK, challenge).into_response()),
        // No metric: the delivery worker records the outcome of the work that
        // was just admitted, and recording one here would double-count it.
        Ok(Received::Delivery(Outcome::Processed { event_type })) => {
            debug!(event_type, "Slack webhook admitted for routing");
            Ok(StatusCode::OK.into_response())
        }
        Ok(Received::Delivery(Outcome::Ignored {
            event_type,
            reason,
            subscription_id,
            connection_id,
        })) => {
            record_webhook(
                "slack",
                "ignored",
                reason,
                &event_type,
                &subscription_id,
                &connection_id,
            );
            Ok(StatusCode::OK.into_response())
        }
        // Nothing durable was written, so a 200 would tell Slack the work is
        // ours while no record of it exists anywhere. Surfaced as `503` at
        // capacity and `500` on a failed write; Slack re-delivers either way.
        Err(IngressError::NotAdmitted(e)) => Err(AdapterError::from(e)),
        // No retry supplies what the request was missing.
        Err(IngressError::Unidentifiable(message)) => {
            Err(SharedAdapterError::BadRequest(message).into())
        }
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

    use moatless_adapter_model::delivery::AdmitError;
    use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
    use moatless_adapter_model::ingress::IngressError;
    use moatless_adapter_model::test_support::response_snapshot;
    use moatless_types::store::StoreError;

    /// The acknowledgment class of each failure the Slack webhook path raises.
    /// An ingress rejection and an admission failure both leave the request
    /// unacknowledged — Slack re-delivers, and nothing durable was lost.
    ///
    /// Every row is reachable: `receive` admits through `admit_delivery`, which
    /// hands back `AdmitError` unflattened, so both `NotAdmitted` rows are
    /// states a real at-capacity or failed write produces.
    #[tokio::test]
    async fn no_failure_on_the_slack_webhook_path_is_acknowledged() {
        for (error, expected) in [
            (
                IngressError::Ingress(SharedAdapterError::SignatureInvalid.into()),
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
                IngressError::NotAdmitted(AdmitError::Store(StoreError::new(
                    "persist the Adapter delivery record",
                ))),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            // The mapping the handler applies, spelled out on its own: the
            // handler itself needs an `AppState` to reach.
            let mapped = match error {
                IngressError::NotAdmitted(e) => AdapterError::from(e),
                IngressError::Unidentifiable(message) => {
                    SharedAdapterError::BadRequest(message).into()
                }
                IngressError::Ingress(e) => e,
            };
            let (status, _) = response_snapshot(mapped.into_response()).await;
            assert_eq!(status, expected);
            assert_ne!(status, StatusCode::OK, "a failure must not acknowledge");
        }
    }
}
