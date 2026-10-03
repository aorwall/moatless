//! The one GitHub proxy route, streamed in both directions.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use tracing::{info, warn};

use crate::github_proxy::service::{GitHubProxy, GitHubProxyError, GitHubUpstream, SandboxScope};

/// `ANY /github/{upstream}/{path}` — one `gh` request, forwarded to
/// `upstream` with the sandbox's GitHub token in place of the task token.
pub async fn proxy(
    State(proxy): State<Arc<GitHubProxy>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some((upstream, path_and_query)) = split_upstream(&uri) else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(
                serde_json::json!({ "message": "Only api and uploads are proxied to GitHub" }),
            ),
        )
            .into_response();
    };
    let scope = match proxy.authenticate(bearer_token(&headers)).await {
        Ok(scope) => scope,
        Err(e) => return e.into_response(),
    };
    forward(
        &proxy,
        &scope,
        upstream,
        method,
        &path_and_query,
        &headers,
        body,
    )
    .await
}

/// The upstream `/github/{upstream}/…` names, and the path and query to
/// request from it, or `None` for an upstream this proxy does not forward to.
fn split_upstream(uri: &Uri) -> Option<(GitHubUpstream, String)> {
    let rest = uri.path().strip_prefix("/github/")?;
    let (segment, path) = rest.split_once('/').unwrap_or((rest, ""));
    let upstream = GitHubUpstream::named(segment)?;
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    Some((upstream, format!("/{path}{query}")))
}

/// Send one request upstream as `scope`'s GitHub identity.
pub(super) async fn forward(
    proxy: &GitHubProxy,
    scope: &SandboxScope,
    upstream: GitHubUpstream,
    method: Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let token = match proxy.token(scope).await {
        Ok(token) => token,
        Err(e) => return e.into_response(),
    };
    let url = proxy.upstream_url(upstream, path_and_query);
    let mut request = proxy
        .http
        .request(method.clone(), &url)
        .body(reqwest::Body::wrap_stream(
            body.into_data_stream().map_err(std::io::Error::other),
        ));
    for (name, value) in headers {
        if forwards(name) && name != header::AUTHORIZATION {
            request = request.header(name, value);
        }
    }
    request = request.bearer_auth(token);

    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            warn!(%error, ?upstream, "A proxied GitHub request did not reach GitHub");
            return GitHubProxyError::Upstream("GitHub is unreachable".to_string()).into_response();
        }
    };
    let status = StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    info!(
        sandbox_id = %scope.sandbox,
        %method,
        ?upstream,
        path = path_and_query.split('?').next().unwrap_or_default(),
        status = status.as_u16(),
        "Proxied a GitHub request"
    );

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
            warn!(%error, "Failed to build a proxied GitHub response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

/// Whether a header crosses the proxy. Hop-by-hop headers describe one
/// connection, and `Host` names the backend rather than GitHub.
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

/// The token of an `Authorization: Bearer` header, which is how the sandbox's
/// `github.sock` presents the pod's task token.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

impl IntoResponse for GitHubProxyError {
    /// A GitHub-shaped error body: `gh` prints its `message`.
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::NoGitHubIdentity(_) => StatusCode::FORBIDDEN,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        (
            status,
            axum::Json(serde_json::json!({ "message": self.to_string() })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests;
