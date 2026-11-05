//! Forwarding one model request upstream and streaming the answer back.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, Method, header};
use axum::response::Response;
use futures::TryStreamExt;

use crate::proxy::forward::{forwards, stream_back};

/// An upstream request, before the credential is attached: the caller's
/// headers minus hop-by-hop ones and its credentials, and its body streamed.
///
/// A `GET` carries no body: a streamed one would add a chunked encoding the
/// upstream may refuse on a bodiless method.
pub fn upstream_request(
    http: &reqwest::Client,
    method: Method,
    url: &str,
    headers: &HeaderMap,
    body: Body,
) -> reqwest::RequestBuilder {
    let mut request = http.request(method.clone(), url);
    if method != Method::GET {
        request = request.body(reqwest::Body::wrap_stream(
            body.into_data_stream().map_err(std::io::Error::other),
        ));
    }
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
    Ok(stream_back(request.send().await?, "model_gateway"))
}

/// The headers a caller presents its task token in, or that name an account
/// the gateway alone chooses, which never go upstream.
fn carries_credential(name: &HeaderName) -> bool {
    name == header::AUTHORIZATION || matches!(name.as_str(), "x-api-key" | "chatgpt-account-id")
}

/// Headers about the caller's session or network path, which are the
/// backend's business rather than the provider's.
fn describes_the_caller(name: &HeaderName) -> bool {
    let name = name.as_str();
    matches!(
        name,
        "cookie" | "forwarded" | "x-real-ip" | "x-moatless-auth-mode"
    ) || name.starts_with("x-forwarded-")
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
