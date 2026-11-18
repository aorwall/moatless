//! The one Forgejo proxy route, streamed in both directions.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use tracing::{info, warn};

use crate::forgejo_proxy::service::{self, ForgejoProxy, ForgejoProxyError};
use crate::proxy::caller::SandboxScope;
use crate::proxy::forward::{bearer_token, forwards, stream_back};

/// `ANY /forgejo/{host}/api/v1/{path}` — one API request, forwarded to `host`
/// with the run-as identity's Forgejo token in place of the task token.
pub async fn proxy(
    State(proxy): State<Arc<ForgejoProxy>>,
    Path((host, _)): Path<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let host = match service::host(&host) {
        Ok(host) => host,
        Err(e) => return e.into_response(),
    };
    let scope = match proxy.authenticate(task_token(&headers)).await {
        Ok(scope) => scope,
        Err(e) => return e.into_response(),
    };
    let Some(path_and_query) = upstream_path(&uri) else {
        return ForgejoProxyError::Refused("No path after the host".to_string()).into_response();
    };
    forward(
        &proxy,
        &scope,
        &host,
        method,
        &path_and_query,
        &headers,
        body,
    )
    .await
}

/// The pod's task token: a relay's `Authorization: Bearer`, else the
/// `X-User-Token` that `moat api` sends on every request.
fn task_token(headers: &HeaderMap) -> Option<&str> {
    bearer_token(headers).or_else(|| headers.get(USER_TOKEN)?.to_str().ok())
}

const USER_TOKEN: &str = "x-user-token";

/// The path and query after `/forgejo/{host}`, which starts with `/`.
fn upstream_path(uri: &Uri) -> Option<String> {
    let rest = uri.path().strip_prefix("/forgejo/")?;
    let (_, path) = rest.split_once('/')?;
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    Some(format!("/{path}{query}"))
}

/// Send one request to `host` as `scope`'s Forgejo identity.
pub(super) async fn forward(
    proxy: &ForgejoProxy,
    scope: &SandboxScope,
    host: &str,
    method: Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    if let Err(e) = service::checked(path_and_query) {
        warn!(sandbox_id = %scope.sandbox, host, %method, path = path_and_query.split('?').next().unwrap_or_default(), "Refused a Forgejo request");
        return e.into_response();
    }
    let token = match proxy.token(scope, host).await {
        Ok(token) => token,
        Err(e) => return e.into_response(),
    };
    let mut request = proxy
        .http
        .request(method.clone(), proxy.upstream_url(host, path_and_query))
        .body(reqwest::Body::wrap_stream(
            body.into_data_stream().map_err(std::io::Error::other),
        ));
    for (name, value) in headers {
        if forwards(name) && !withheld(name) {
            request = request.header(name, value);
        }
    }
    let response = match request
        .header(header::AUTHORIZATION, format!("token {token}"))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            warn!(%error, host, "A proxied Forgejo request did not reach Forgejo");
            return ForgejoProxyError::Upstream(format!("Forgejo at {host} is unreachable"))
                .into_response();
        }
    };
    info!(
        sandbox_id = %scope.sandbox,
        host,
        method = %method,
        path = path_and_query.split('?').next().unwrap_or_default(),
        status = response.status().as_u16(),
        "Proxied a Forgejo request"
    );
    stream_back(response, "forgejo")
}

/// Headers the pod sends that never reach Forgejo: either task token, a cookie
/// that could authenticate as someone else, and `Sudo` (see
/// [`service::checked`]).
fn withheld(name: &header::HeaderName) -> bool {
    name == header::AUTHORIZATION
        || name == header::COOKIE
        || name.as_str() == USER_TOKEN
        || name.as_str() == "sudo"
}

impl IntoResponse for ForgejoProxyError {
    /// A Forgejo-shaped error body: its clients print `message`.
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Refused(_) | Self::NoCredential(_) => StatusCode::FORBIDDEN,
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
