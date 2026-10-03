//! The unix socket `gh` sends every request over, through the
//! `http_unix_socket` setting [`super::gh_config`] writes.
//!
//! `gh` speaks plain HTTP on the socket and names the host it meant in `Host`.
//! A GitHub API host goes to the backend's GitHub proxy with the pod's task
//! token, and the backend presents the run-as identity's live GitHub token;
//! any other host (the signed storage a log download redirects to) is fetched
//! directly. No GitHub token enters the pod on this path.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use moatless_types::http::HttpStreamTimeouts;
use tracing::warn;

use crate::backend_client::BackendClient;

/// Shared state for the proxy handler.
#[derive(Clone)]
pub struct GitHubProxyState {
    /// Holds the task token the backend authenticates the pod with.
    pub backend_client: Arc<dyn BackendClient>,
    /// `BACKEND_API_URL` without a trailing `/`, which the git proxy's clone
    /// URLs also hang off. `None` refuses every GitHub API request.
    pub backend_url: Option<String>,
    pub http: reqwest::Client,
}

impl GitHubProxyState {
    /// Idle, not total: a log or release asset streams for as long as it is
    /// big.
    ///
    /// Redirects are returned to `gh`, which follows them back through this
    /// socket, so each hop is routed by its own host.
    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub fn new(
        backend_client: Arc<dyn BackendClient>,
        backend_url: Option<&str>,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .read_timeout(timeouts.idle)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("the TLS backend initialises");
        Self {
            backend_client,
            backend_url: backend_url
                .filter(|url| !url.is_empty())
                .map(|url| url.trim_end_matches('/').to_string()),
            http,
        }
    }
}

/// Where one request from `gh` goes.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// The backend's GitHub proxy, which needs the task token.
    Backend(String),
    /// The host `gh` named, reached directly and with no credential.
    Direct(String),
}

/// Route a request for `host` to the backend when it is a GitHub API host,
/// and straight to `host` otherwise. `None` when the API host cannot be
/// reached because the backend's address is unknown.
fn target(host: &str, path_and_query: &str, backend_url: Option<&str>) -> Option<Target> {
    let host = host.strip_suffix(":443").unwrap_or(host);
    let upstream = match host {
        "api.github.com" => "api",
        "uploads.github.com" => "uploads",
        _ => return Some(Target::Direct(format!("https://{host}{path_and_query}"))),
    };
    let backend_url = backend_url?;
    Some(Target::Backend(format!(
        "{backend_url}/github/{upstream}{path_and_query}"
    )))
}

async fn forward(State(state): State<GitHubProxyState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Some(host) = parts
        .headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| parts.uri.host())
    else {
        return error(StatusCode::BAD_REQUEST, "The request names no host");
    };
    let path_and_query = parts
        .uri
        .path_and_query()
        .map_or("/", |path_and_query| path_and_query.as_str());
    let Some(target) = target(host, path_and_query, state.backend_url.as_deref()) else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "This sandbox has no BACKEND_API_URL, so it cannot reach GitHub's API",
        );
    };

    let (url, task_token) = match target {
        Target::Backend(url) => match state.backend_client.auth_token() {
            Some(token) => (url, Some(token)),
            None => {
                return error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "This sandbox holds no task token yet; retry once it has started",
                );
            }
        },
        Target::Direct(url) => (url, None),
    };

    let mut upstream = state
        .http
        .request(parts.method, &url)
        .body(reqwest::Body::wrap_stream(
            body.into_data_stream().map_err(std::io::Error::other),
        ));
    for (name, value) in &parts.headers {
        // `gh`'s own `Authorization` carries the placeholder `gh_config`
        // writes, never a credential worth sending anywhere.
        if forwards(name) && name != header::AUTHORIZATION {
            upstream = upstream.header(name, value);
        }
    }
    if let Some(token) = task_token {
        upstream = upstream.bearer_auth(token);
    }

    let response = match upstream.send().await {
        Ok(response) => response,
        Err(e) => {
            warn!(error = %e, host, "A gh request did not reach its host");
            return error(StatusCode::BAD_GATEWAY, &format!("{host} is unreachable"));
        }
    };
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
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
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to build a gh response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

/// Whether a header crosses the proxy. Hop-by-hop headers describe one
/// connection, and `Host` is the socket's, rebuilt from the target URL.
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

/// A GitHub-shaped error body: `gh` prints its `message`.
fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({ "message": message })),
    )
        .into_response()
}

/// Every path, since `gh` asks for whatever its command needs.
pub fn router(state: GitHubProxyState) -> Router {
    Router::new().fallback(forward).with_state(state)
}

#[cfg(test)]
mod tests;
