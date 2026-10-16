//! HTTP request tracing and correlation headers.

use moatless_types::http::RelayedUpstreamFailure;

pub(crate) async fn request_trace_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use tracing::Instrument;

    let method = req.method().clone();
    let path = req.uri().path().to_string();
    // K8s liveness/readiness probes hit these on a tight interval; access-logging
    // them would recreate the high-volume probe noise we are trying to remove.
    let is_probe = matches!(path.as_str(), "/health" | "/healthz" | "/ready");
    let request_id = uuid::Uuid::new_v4().to_string();

    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %path,
        user_id = tracing::field::Empty,
        status = tracing::field::Empty,
        latency_ms = tracing::field::Empty,
    );

    let start = std::time::Instant::now();
    async move {
        let mut res = next.run(req).await;
        let latency_ms = start.elapsed().as_millis() as u64;
        let span = tracing::Span::current();
        span.record("status", res.status().as_u16());
        span.record("latency_ms", latency_ms);
        if !is_probe {
            match completion_level(&res) {
                tracing::Level::ERROR => tracing::error!("request completed"),
                tracing::Level::WARN => tracing::warn!("request completed"),
                _ => tracing::debug!("request completed"),
            }
        }
        if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
            res.headers_mut().insert("x-request-id", value);
        }
        res
    }
    .instrument(span)
    .await
}

/// Successful requests are high-volume access-log noise, so they log at DEBUG.
/// A 5xx logs at ERROR unless the handler marked it a
/// [`RelayedUpstreamFailure`], which logs at WARN like a 4xx.
fn completion_level(res: &axum::response::Response) -> tracing::Level {
    let status = res.status();
    let relayed = res.extensions().get::<RelayedUpstreamFailure>().is_some();
    if status.is_server_error() && !relayed {
        tracing::Level::ERROR
    } else if status.is_server_error() || status.is_client_error() {
        tracing::Level::WARN
    } else {
        tracing::Level::DEBUG
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, response::IntoResponse};

    #[test]
    fn a_backend_5xx_logs_at_error() {
        let res = StatusCode::INTERNAL_SERVER_ERROR.into_response();
        assert_eq!(completion_level(&res), tracing::Level::ERROR);
    }

    #[test]
    fn a_relayed_upstream_5xx_logs_at_warn() {
        let mut res = StatusCode::BAD_GATEWAY.into_response();
        res.extensions_mut().insert(RelayedUpstreamFailure);
        assert_eq!(completion_level(&res), tracing::Level::WARN);
    }

    #[test]
    fn a_4xx_logs_at_warn_and_a_success_at_debug() {
        let res = StatusCode::BAD_REQUEST.into_response();
        assert_eq!(completion_level(&res), tracing::Level::WARN);
        let res = StatusCode::NO_CONTENT.into_response();
        assert_eq!(completion_level(&res), tracing::Level::DEBUG);
    }
}
