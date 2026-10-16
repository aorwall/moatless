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
use moatless_types::http::{HttpTimeouts, RelayedUpstreamFailure};
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use std::sync::Arc;
use tracing::warn;

use crate::UiRpcState;

/// The route the T3 client posts to. Fixed by the client, not chosen here.
pub(crate) const TRACES_ROUTE: &str = "/api/observability/v1/traces";

/// Client trace batches by `outcome`: `forwarded`, `dropped` (no collector
/// configured), `unreadable`, `refused` (collector answered non-2xx) or
/// `unreachable`.
const CLIENT_TRACES: &str = "moatless_ui_rpc_client_trace_batches_total";

/// The largest batch accepted, matching the 4 MiB the collector's own gRPC
/// receiver caps a message at — a larger body could not be forwarded anyway.
const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

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
    /// Forward to `endpoint`, and describe the forwarding metric.
    pub(crate) fn configured(endpoint: Option<String>, timeouts: HttpTimeouts) -> Self {
        metrics::describe_counter!(
            CLIENT_TRACES,
            "T3 client OTLP trace batches received, by outcome"
        );
        Self::new(endpoint, timeouts)
    }

    /// `timeouts` bounds how long a browser's export can hold a backend task:
    /// the client drops what it could not send, so a slow collector must not
    /// become a slow request.
    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub(crate) fn new(endpoint: Option<String>, timeouts: HttpTimeouts) -> Self {
        Self {
            endpoint: endpoint
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(Arc::from),
            client: reqwest::Client::builder()
                .connect_timeout(timeouts.connect)
                .timeout(timeouts.total)
                .build()
                .expect("the TLS backend initialises"),
        }
    }

    /// The value reported to the client as `observability.otlpTracesUrl`, so
    /// the Settings panel names where its own spans go.
    pub(crate) fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }
}

/// Take one OTLP/JSON batch from the browser and forward it.
///
/// Answers `204` whether forwarded or collected by nobody, and `502` only when
/// a configured collector failed it — the one case worth telling the client,
/// whose exporter then backs off.
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
            metrics::counter!(CLIENT_TRACES, "outcome" => "unreadable").increment(1);
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
        .body(batch)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            metrics::counter!(CLIENT_TRACES, "outcome" => "forwarded").increment(1);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(response) => {
            metrics::counter!(CLIENT_TRACES, "outcome" => "refused").increment(1);
            warn!(
                status = %response.status(),
                endpoint,
                "The trace collector refused a T3 client trace batch"
            );
            export_failed()
        }
        Err(error) => {
            metrics::counter!(CLIENT_TRACES, "outcome" => "unreachable").increment(1);
            warn!(
                %error,
                endpoint,
                "Could not reach the trace collector with a T3 client trace batch"
            );
            export_failed()
        }
    }
}

/// The collector's failure, relayed to the client as `502` and marked so
/// request tracing does not log a telemetry outage as a backend error.
fn export_failed() -> Response {
    let mut response = (StatusCode::BAD_GATEWAY, "Trace export failed.").into_response();
    response.extensions_mut().insert(RelayedUpstreamFailure);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forwarder(endpoint: Option<String>) -> ClientTraceForwarder {
        ClientTraceForwarder::new(
            endpoint,
            moatless_core::config::HttpConfig::default().client_traces,
        )
    }

    #[test]
    fn an_unset_endpoint_collects_nothing() {
        assert_eq!(forwarder(None).endpoint(), None);
    }

    /// An empty Helm value reaches a container as an empty or whitespace-only
    /// variable, and that has to read as off rather than as a URL to POST to —
    /// otherwise every batch fails against an unparseable endpoint.
    #[test]
    fn a_blank_endpoint_reads_as_unset() {
        assert_eq!(forwarder(Some(String::new())).endpoint(), None);
        assert_eq!(forwarder(Some("  \n ".to_string())).endpoint(), None);
    }

    #[test]
    fn a_configured_endpoint_is_trimmed_and_reported() {
        assert_eq!(
            forwarder(Some(" http://otel-collector:4318/v1/traces ".to_string())).endpoint(),
            Some("http://otel-collector:4318/v1/traces")
        );
    }

    #[test]
    fn a_failed_export_is_a_502_marked_as_relayed() {
        let response = export_failed();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(
            response
                .extensions()
                .get::<RelayedUpstreamFailure>()
                .is_some()
        );
    }
}
