//! HTTP request tracing and correlation headers.

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
            // Successful requests are high-volume access-log noise → log at
            // DEBUG. Escalate failures so they stay visible by default: 5xx at
            // ERROR, 4xx at WARN.
            if res.status().is_server_error() {
                tracing::error!("request completed");
            } else if res.status().is_client_error() {
                tracing::warn!("request completed");
            } else {
                tracing::debug!("request completed");
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
