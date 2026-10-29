//! Forwarding one model request upstream and streaming the answer back.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use tracing::warn;

/// An upstream request, before the credential is attached: the caller's
/// headers minus hop-by-hop ones and its task token, and its body streamed.
pub fn upstream_request(
    http: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    body: Body,
) -> reqwest::RequestBuilder {
    let mut request = http.post(url).body(reqwest::Body::wrap_stream(
        body.into_data_stream().map_err(std::io::Error::other),
    ));
    for (name, value) in headers {
        if forwards(name) && !carries_credential(name) && !describes_the_caller(name) {
            request = request.header(name, value);
        }
    }
    request
}

/// Send `request` and stream the upstream's status, headers and body back
/// unbuffered, so an SSE event reaches the agent as the provider sends it and
/// a `429` keeps its rate-limit headers.
pub async fn relay(request: reqwest::RequestBuilder) -> Result<Response, reqwest::Error> {
    let response = request.send().await?;
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
    Ok(out
        .body(Body::from_stream(response.bytes_stream()))
        .unwrap_or_else(|error| {
            warn!(%error, "Failed to build a model gateway response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }))
}

/// Whether a header crosses the gateway. Hop-by-hop headers describe one
/// connection, and `Host` names the backend rather than the provider.
fn forwards(name: &HeaderName) -> bool {
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

/// The headers a caller presents its task token in, which never go upstream.
fn carries_credential(name: &HeaderName) -> bool {
    name == header::AUTHORIZATION || name.as_str() == "x-api-key"
}

/// Headers about the caller's session or network path, which are the
/// backend's business rather than the provider's.
fn describes_the_caller(name: &HeaderName) -> bool {
    let name = name.as_str();
    matches!(name, "cookie" | "forwarded" | "x-real-ip") || name.starts_with("x-forwarded-")
}

/// The task token a request presents: `x-api-key`, which Claude sends an
/// `ANTHROPIC_API_KEY` as, else an `Authorization: Bearer` token.
pub fn presented_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        return Some(key.trim());
    }
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}
