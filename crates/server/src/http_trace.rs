//! HTTP request tracing and correlation headers.

use std::borrow::Cow;

use moatless_types::http::{RefusedAtCapacity, RelayedUpstreamFailure};

/// Path prefixes whose next segment is a page or attachment upload token.
/// Copies of `PAGE_RECORDS_ROUTE`'s and `UPLOAD_ROUTE_PREFIX`'s prefixes: a
/// route renamed without this list writes its token to every request log.
const TOKEN_PATH_PREFIXES: [&str; 2] = ["/api/artifact-data/", "/api/attachments/upload/"];

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
        path = %redact_path(&path),
        user_id = tracing::field::Empty,
        status = tracing::field::Empty,
        latency_ms = tracing::field::Empty,
    );

    let start = std::time::Instant::now();
    async move {
        let mut res = next.run(req).await;
        let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
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

/// Replaces the token segment after a [`TOKEN_PATH_PREFIXES`] entry with
/// `:token`, keeping any suffix such as `/records` or `/ws`.
fn redact_path(path: &str) -> Cow<'_, str> {
    for prefix in TOKEN_PATH_PREFIXES {
        if let Some(rest) = path.strip_prefix(prefix) {
            let suffix = rest.find('/').map_or("", |at| &rest[at..]);
            return Cow::Owned(format!("{prefix}:token{suffix}"));
        }
    }
    Cow::Borrowed(path)
}

/// Successful requests are high-volume access-log noise, so they log at DEBUG.
/// A 5xx logs at ERROR unless the handler marked it a
/// [`RelayedUpstreamFailure`] or [`RefusedAtCapacity`], which log at WARN like
/// a 4xx.
fn completion_level(res: &axum::response::Response) -> tracing::Level {
    let status = res.status();
    let extensions = res.extensions();
    let marked = extensions.get::<RelayedUpstreamFailure>().is_some()
        || extensions.get::<RefusedAtCapacity>().is_some();
    if status.is_server_error() && !marked {
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
    fn a_page_token_is_redacted_on_every_artifact_data_route() {
        assert_eq!(
            redact_path("/api/artifact-data/abc.def/records"),
            "/api/artifact-data/:token/records"
        );
        assert_eq!(
            redact_path("/api/artifact-data/abc.def/records/vote%2F1"),
            "/api/artifact-data/:token/records/vote%2F1"
        );
        assert_eq!(
            redact_path("/api/artifact-data/abc.def/ws"),
            "/api/artifact-data/:token/ws"
        );
        assert_eq!(
            redact_path("/api/artifact-data/abc.def"),
            "/api/artifact-data/:token"
        );
    }

    #[test]
    fn an_attachment_upload_token_is_redacted() {
        assert_eq!(
            redact_path("/api/attachments/upload/abc.def"),
            "/api/attachments/upload/:token"
        );
    }

    #[test]
    fn a_path_without_a_token_is_borrowed_unchanged() {
        for path in ["/api/v1/tasks/1", "/api/artifact-data", "/health"] {
            assert!(matches!(redact_path(path), Cow::Borrowed(p) if p == path));
        }
    }

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
    fn a_503_refused_at_capacity_logs_at_warn() {
        let mut res = StatusCode::SERVICE_UNAVAILABLE.into_response();
        res.extensions_mut().insert(RefusedAtCapacity);
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
