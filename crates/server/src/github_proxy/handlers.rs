//! The one GitHub proxy route, streamed in both directions.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use tracing::{info, warn};

use crate::github_proxy::service::{GitHubProxy, GitHubProxyError, GitHubUpstream};
use crate::github_proxy::target;
use crate::proxy::caller::SandboxScope;
use crate::proxy::forward::{bearer_token, forwards, stream_back};

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
    let (requested, body) = if is_graphql(upstream, &method, path_and_query) {
        let Ok(bytes) = axum::body::to_bytes(body, GRAPHQL_BODY_LIMIT).await else {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                axum::Json(
                    serde_json::json!({ "message": "The GraphQL request body is too large" }),
                ),
            )
                .into_response();
        };
        (target::from_graphql(&bytes), reqwest::Body::from(bytes))
    } else {
        (
            target::from_path(path_and_query),
            reqwest::Body::wrap_stream(body.into_data_stream().map_err(std::io::Error::other)),
        )
    };
    let token = match proxy.token(scope, requested.as_ref()).await {
        Ok(token) => token,
        Err(e) => return e.into_response(),
    };
    let url = proxy.upstream_url(upstream, path_and_query);
    let mut request = proxy.http.request(method.clone(), &url).body(body);
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
    info!(
        sandbox_id = %scope.sandbox,
        %method,
        ?upstream,
        path = path_and_query.split('?').next().unwrap_or_default(),
        status = response.status().as_u16(),
        "Proxied a GitHub request"
    );
    stream_back(response, "github")
}

/// Bounds the GraphQL body buffered to read the repository its variables
/// name. A mutation carrying a pull request body is the largest `gh` sends.
const GRAPHQL_BODY_LIMIT: usize = 4 * 1024 * 1024;

fn is_graphql(upstream: GitHubUpstream, method: &Method, path_and_query: &str) -> bool {
    upstream == GitHubUpstream::Api
        && method == Method::POST
        && path_and_query.split('?').next() == Some("/graphql")
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
