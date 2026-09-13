//! The T3 client's own spans, forwarded to a collector.
//!
//! T3's web client posts OTLP/JSON to `/api/observability/v1/traces`
//! (`apps/web/src/observability/clientTracing.ts`). Forwarding that body to the
//! same collector this process exports to makes a request one trace rather than
//! two: the client's span is the parent
//! [`crate::rpc::metrics::request_root_span`] adopts, and an unexported parent
//! leaves the backend's spans dangling in Tempo.

use axum::{
    body::Body,
    extract::{Extension, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use std::{sync::Arc, time::Duration};
use tracing::warn;

use crate::UiRpcState;

/// The route the T3 client posts to. Fixed by the client, not chosen here.
pub(crate) const TRACES_ROUTE: &str = "/api/observability/v1/traces";

/// Client trace batches by `outcome` (`forwarded` | `dropped` | `failed`).
const CLIENT_TRACES: &str = "moatless_ui_rpc_client_trace_batches_total";

/// The largest batch accepted, matching the 4 MiB the collector's own gRPC
/// receiver caps a message at — a larger body could not be forwarded anyway.
const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

/// Bounds how long a browser's export can hold a backend task. The client
/// batches on its own interval and drops what it could not send, so a slow
/// collector must not become a slow request.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);

/// Where a client's spans go, or `None` when this deployment collects none.
///
/// `None` is not an error told to the client: it keeps exporting and this
/// backend silently drops the batches — that is what "off" means here.
#[derive(Clone)]
pub(crate) struct ClientTraceForwarder {
    endpoint: Option<Arc<str>>,
    client: reqwest::Client,
}

impl std::fmt::Debug for ClientTraceForwarder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientTraceForwarder")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl ClientTraceForwarder {
    /// Read the endpoint from `T3_CLIENT_OTLP_TRACES_ENDPOINT`, which names an
    /// OTLP/HTTP traces URL such as `http://otel-collector:4318/v1/traces`.
    pub(crate) fn from_env() -> Self {
        metrics::describe_counter!(
            CLIENT_TRACES,
            "T3 client OTLP trace batches received, by outcome"
        );
        Self::new(std::env::var("T3_CLIENT_OTLP_TRACES_ENDPOINT").ok())
    }

    fn new(endpoint: Option<String>) -> Self {
        Self {
            endpoint: endpoint
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(Arc::from),
            client: reqwest::Client::new(),
        }
    }

    /// The value reported to the client as `observability.otlpTracesUrl`, so
    /// the Settings panel names where its own spans go.
    pub(crate) fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }
}

impl Default for ClientTraceForwarder {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Take one OTLP/JSON batch from the browser and forward it.
///
/// Answers `204` whether forwarded or collected by nobody, and `502` only when
/// a configured collector refused it — the one case worth telling the client.
pub(crate) async fn receive(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    body: Body,
) -> Response {
    let forwarder = &state.client_traces;
    let Some(endpoint) = forwarder.endpoint.as_deref() else {
        metrics::counter!(CLIENT_TRACES, "outcome" => "dropped").increment(1);
        return StatusCode::NO_CONTENT.into_response();
    };

    let batch = match axum::body::to_bytes(body, MAX_BATCH_BYTES).await {
        Ok(batch) => batch,
        Err(error) => {
            metrics::counter!(CLIENT_TRACES, "outcome" => "failed").increment(1);
            warn!(
                %error,
                user_id = %identity.user_id,
                "Rejected a T3 client trace batch this backend could not read"
            );
            return (StatusCode::BAD_REQUEST, "Trace batch unreadable.").into_response();
        }
    };

    match forwarder
        .client
        .post(endpoint)
        .header(header::CONTENT_TYPE, "application/json")
        .timeout(FORWARD_TIMEOUT)
        .body(batch)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            metrics::counter!(CLIENT_TRACES, "outcome" => "forwarded").increment(1);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(response) => {
            metrics::counter!(CLIENT_TRACES, "outcome" => "failed").increment(1);
            warn!(
                status = %response.status(),
                endpoint,
                "The trace collector refused a T3 client trace batch"
            );
            (StatusCode::BAD_GATEWAY, "Trace export failed.").into_response()
        }
        Err(error) => {
            metrics::counter!(CLIENT_TRACES, "outcome" => "failed").increment(1);
            warn!(
                %error,
                endpoint,
                "Could not reach the trace collector with a T3 client trace batch"
            );
            (StatusCode::BAD_GATEWAY, "Trace export failed.").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_endpoint_collects_nothing() {
        assert_eq!(ClientTraceForwarder::default().endpoint(), None);
        assert_eq!(ClientTraceForwarder::new(None).endpoint(), None);
    }

    /// An empty Helm value reaches a container as an empty or whitespace-only
    /// variable, and that has to read as off rather than as a URL to POST to —
    /// otherwise every batch fails against an unparseable endpoint.
    #[test]
    fn a_blank_endpoint_reads_as_unset() {
        assert_eq!(
            ClientTraceForwarder::new(Some(String::new())).endpoint(),
            None
        );
        assert_eq!(
            ClientTraceForwarder::new(Some("  \n ".to_string())).endpoint(),
            None
        );
    }

    #[test]
    fn a_configured_endpoint_is_trimmed_and_reported() {
        assert_eq!(
            ClientTraceForwarder::new(Some(" http://otel-collector:4318/v1/traces ".to_string()))
                .endpoint(),
            Some("http://otel-collector:4318/v1/traces")
        );
    }
}
