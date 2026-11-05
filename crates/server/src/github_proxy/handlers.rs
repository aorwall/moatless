//! The one GitHub proxy route, streamed in both directions.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures::{StreamExt, TryStreamExt};
use tracing::{info, warn};

use moatless_core::proxy_rules::NormalPath;

use crate::github_policy::{GitHubPolicy, Refusal, Request as PolicyRequest};
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
            return too_large("The GraphQL request body is too large");
        };
        if let Err(refusal) = proxy.policy.check_graphql(&scope.workspace, &bytes).await {
            if let Some(refused) = refuse(proxy, scope, &method, path_and_query, &refusal) {
                return refused;
            }
        }
        (target::from_graphql(&bytes), reqwest::Body::from(bytes))
    } else if !GitHubPolicy::is_write(method.as_str()) {
        (
            target::from_path(path_and_query),
            reqwest::Body::wrap_stream(body.into_data_stream().map_err(std::io::Error::other)),
        )
    } else {
        match check_rest(proxy, scope, &method, path_and_query, body).await {
            Ok(body) => (target::from_path(path_and_query), body),
            Err(refused) => return *refused,
        }
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

/// Run a REST write past the policy, buffering its body only when the policy
/// reads it.
async fn check_rest(
    proxy: &GitHubProxy,
    scope: &SandboxScope,
    method: &Method,
    path_and_query: &str,
    body: Body,
) -> Result<reqwest::Body, Box<Response>> {
    let path = match NormalPath::parse(path_and_query) {
        Ok(path) => path,
        Err(e) => {
            let refusal = Refusal::new("bad_path", format!("Refused {path_and_query}: {e}"));
            return match refuse(proxy, scope, method, path_and_query, &refusal) {
                Some(refused) => Err(Box::new(refused)),
                None => Ok(stream(body)),
            };
        }
    };
    let (decision, body) = if GitHubPolicy::rest_needs_body(method.as_str(), &path) {
        let bytes = match buffer(body, REVIEW_BODY_LIMIT).await {
            Buffered::Whole(bytes) => bytes,
            Buffered::Failed => return Err(Box::new(StatusCode::BAD_REQUEST.into_response())),
            Buffered::Over(body) => {
                let refusal = Refusal::new(
                    "review_unreadable",
                    "The review body is over 1 MiB, too large for the GitHub proxy to read its event",
                );
                return match refuse(proxy, scope, method, path_and_query, &refusal) {
                    Some(refused) => Err(Box::new(refused)),
                    None => Ok(body),
                };
            }
        };
        let decision = proxy
            .policy
            .check_rest(&scope.workspace, method.as_str(), &path, Some(&bytes))
            .await;
        (decision, reqwest::Body::from(bytes))
    } else {
        let decision = proxy
            .policy
            .check_rest(&scope.workspace, method.as_str(), &path, None)
            .await;
        (decision, stream(body))
    };
    match decision {
        Ok(()) => Ok(body),
        Err(refusal) => match refuse(proxy, scope, method, path_and_query, &refusal) {
            Some(refused) => Err(Box::new(refused)),
            None => Ok(body),
        },
    }
}

enum Buffered {
    Whole(Bytes),
    /// Over the limit: what was read, chained ahead of the rest.
    Over(reqwest::Body),
    /// The client's body failed mid-read.
    Failed,
}

/// Read `body` whole when it fits in `limit` bytes.
async fn buffer(body: Body, limit: usize) -> Buffered {
    let mut stream = body.into_data_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return Buffered::Failed;
        };
        buf.extend_from_slice(&chunk);
        if buf.len() > limit {
            let head = futures::stream::once(async move { Ok::<_, axum::Error>(Bytes::from(buf)) });
            return Buffered::Over(reqwest::Body::wrap_stream(
                head.chain(stream).map_err(std::io::Error::other),
            ));
        }
    }
    Buffered::Whole(Bytes::from(buf))
}

fn stream(body: Body) -> reqwest::Body {
    reqwest::Body::wrap_stream(body.into_data_stream().map_err(std::io::Error::other))
}

/// The 403 for `refusal` under `enforce`; `None` under `log`, where the
/// request goes on.
fn refuse(
    proxy: &GitHubProxy,
    scope: &SandboxScope,
    method: &Method,
    path_and_query: &str,
    refusal: &Refusal,
) -> Option<Response> {
    let request = PolicyRequest {
        proxy: "github",
        sandbox_id: &scope.sandbox,
        workspace_id: &scope.workspace,
        method: method.as_str(),
        path: path_and_query
            .split('?')
            .next()
            .unwrap_or_default()
            .to_string(),
    };
    proxy.policy.refuses(&request, refusal).then(|| {
        (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "message": refusal.message })),
        )
            .into_response()
    })
}

fn too_large(message: &str) -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        axum::Json(serde_json::json!({ "message": message })),
    )
        .into_response()
}

/// Bounds a review body buffered to read its `event`.
const REVIEW_BODY_LIMIT: usize = 1024 * 1024;

/// Bounds the GraphQL body buffered to read its operations and the repository
/// its variables name. A mutation carrying a pull request body is the largest
/// `gh` sends.
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
