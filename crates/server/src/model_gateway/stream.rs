//! Forwarding one model request upstream and streaming the answer back.

use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderName, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use moatless_types::http::RelayedUpstreamFailure;
use tracing::info;

use crate::proxy::forward::{forwards, stream_back};

/// An upstream request, before the credential is attached: the caller's
/// headers minus hop-by-hop ones and its credentials, and its [`buffered`]
/// body, which lets [`relay`] send it again.
///
/// A `GET` carries no body, as the upstream may refuse one on a bodiless
/// method.
pub fn upstream_request(
    http: &reqwest::Client,
    method: Method,
    url: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> reqwest::RequestBuilder {
    let mut request = http.request(method.clone(), url);
    if method != Method::GET {
        request = request.body(body);
    }
    for (name, value) in headers {
        if forwards(name) && !carries_credential(name) && !describes_the_caller(name) {
            request = request.header(name, value);
        }
    }
    request
}

/// The largest request body the gateway buffers, above Anthropic's 32 MB
/// Messages limit.
const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;

/// The caller's body in memory.
pub async fn buffered(body: Body) -> Result<Bytes, UnreadBody> {
    axum::body::to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(UnreadBody)
}

/// A request body that failed to arrive or exceeded [`MAX_REQUEST_BYTES`].
pub struct UnreadBody(axum::Error);

impl IntoResponse for UnreadBody {
    fn into_response(self) -> Response {
        let error = self.0.into_inner();
        let status = if error.is::<http_body_util::LengthLimitError>() {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        };
        (
            status,
            format!("The request body could not be read within {MAX_REQUEST_BYTES} bytes: {error}"),
        )
            .into_response()
    }
}

/// How long a request is resent while its upstream refuses the connection.
#[derive(Debug, Clone, Copy)]
pub struct ConnectRetry {
    /// The first wait; each later one doubles, up to [`LONGEST_CONNECT_WAIT`].
    pub first: Duration,
    /// No attempt starts later than this after the first.
    pub budget: Duration,
}

/// Sized for the hub's restart: its single replica sits on a `ReadWriteOnce`
/// volume, and a node replacement left it unreachable for about 35 s. Applies
/// to every upstream, and a connect timeout counts as a refusal.
const HUB_RESTART: ConnectRetry = ConnectRetry {
    first: Duration::from_millis(250),
    budget: Duration::from_secs(60),
};

const LONGEST_CONNECT_WAIT: Duration = Duration::from_secs(5);

/// Send `request` and stream the upstream's status, headers and body back
/// unbuffered, so an SSE event reaches the agent as the provider sends it and
/// a `429` keeps its rate-limit headers.
///
/// A refused connection is retried for [`HUB_RESTART`]; any other failure is
/// not, since the upstream may already have acted on the request.
pub async fn relay(request: reqwest::RequestBuilder) -> Result<Response, reqwest::Error> {
    relay_retrying(request, &HUB_RESTART).await
}

async fn relay_retrying(
    request: reqwest::RequestBuilder,
    retry: &ConnectRetry,
) -> Result<Response, reqwest::Error> {
    let started = Instant::now();
    let mut wait = retry.first;
    let mut attempts = 1u32;
    loop {
        // `None` only for a streamed body, which `upstream_request` never sets.
        let Some(attempt) = request.try_clone() else {
            return Ok(relayed(request.send().await?));
        };
        match attempt.send().await {
            Ok(response) => {
                if attempts > 1 {
                    info!(
                        attempts,
                        "A model request reached its upstream after a retry"
                    );
                }
                return Ok(relayed(response));
            }
            Err(error) if error.is_connect() && started.elapsed() + wait <= retry.budget => {
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(LONGEST_CONNECT_WAIT);
                attempts += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// `response` streamed back, a 5xx marked [`RelayedUpstreamFailure`]: a hub
/// answering 503 while a provider throttles it is the provider's failure, not
/// this backend's, and logging it at ERROR fires the error-burst alert.
fn relayed(response: reqwest::Response) -> Response {
    let mut relayed = stream_back(response, "model_gateway");
    if relayed.status().is_server_error() {
        relayed.extensions_mut().insert(RelayedUpstreamFailure);
    }
    relayed
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

#[cfg(test)]
mod tests;
