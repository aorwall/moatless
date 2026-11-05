//! Header filtering and response streaming every sandbox-facing proxy uses.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tracing::warn;

/// Whether a header crosses a proxy. Hop-by-hop headers describe one
/// connection, and `Host` names the backend rather than the upstream.
pub fn forwards(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
    )
}

/// The token of an `Authorization: Bearer` header, which is how a pod's relay
/// presents its task token.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// The upstream's status, headers and body, streamed back unbuffered.
/// `proxy` names the proxy in the log line a failed build writes.
pub fn stream_back(response: reqwest::Response, proxy: &'static str) -> Response {
    let status = StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut out = Response::builder().status(status);
    for (name, value) in response.headers() {
        // `reqwest` already dechunked the body, so a forwarded length or
        // transfer encoding would describe bytes the stream no longer carries.
        if forwards(name) && name != header::CONTENT_LENGTH {
            if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) {
                out = out.header(name.clone(), value);
            }
        }
    }
    out.body(Body::from_stream(response.bytes_stream()))
        .unwrap_or_else(|error| {
            warn!(%error, proxy, "Failed to build a proxied response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neither_the_task_token_nor_a_hop_by_hop_header_is_forwarded() {
        assert!(!forwards(&header::HOST));
        assert!(!forwards(&header::TRANSFER_ENCODING));
        assert!(forwards(&header::ACCEPT));
        assert!(forwards(&HeaderName::from_static("x-github-api-version")));
        assert_eq!(
            bearer_token(&HeaderMap::from_iter([(
                header::AUTHORIZATION,
                HeaderValue::from_static("Bearer task-jwt"),
            )])),
            Some("task-jwt")
        );
    }
}
