//! The three git smart-HTTP routes, streamed in both directions.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use serde::Deserialize;
use tracing::{info, warn};

use crate::git_proxy::service::{GitProxy, GitProxyError, Upstream};

/// The two services git advertises, and the only two this proxy forwards.
///
/// Anything else is a path git never asks for, and forwarding it would make the
/// proxy a general-purpose fetcher for whatever the credential can reach.
const SERVICES: [&str; 2] = ["git-upload-pack", "git-receive-pack"];

/// Headers copied from git's request onto the upstream one.
///
/// `Content-Encoding` is not optional: git gzips an `upload-pack` request once
/// it is large enough, and a host that is not told so reads the compressed
/// bytes as pkt-line and answers a bare `0002`, which git reports as
/// `fatal: expected 'packfile'`. A small `ls-refs` request is not gzipped, so a
/// clone gets as far as the ref list before it fails.
const FORWARDED_REQUEST_HEADERS: [HeaderName; 4] = [
    header::CONTENT_TYPE,
    header::CONTENT_ENCODING,
    header::ACCEPT,
    header::USER_AGENT,
];

/// Headers copied from the upstream response. `Content-Length` and
/// `Transfer-Encoding` are hop-by-hop and stay out rather than being overlooked:
/// `reqwest` has already dechunked the body this hop re-frames, so either one
/// forwarded describes an encoding the bytes beneath it no longer carry.
const FORWARDED_RESPONSE_HEADERS: [HeaderName; 2] = [header::CONTENT_TYPE, header::CACHE_CONTROL];

#[derive(Deserialize)]
pub struct RefsQuery {
    service: String,
}

/// `GET /git/{repository_id}/info/refs?service=…` — the reference advertisement
/// that opens every clone, fetch and push.
pub async fn info_refs(
    State(proxy): State<Arc<GitProxy>>,
    Path(repository_id): Path<String>,
    Query(query): Query<RefsQuery>,
    headers: HeaderMap,
) -> Response {
    if !SERVICES.contains(&query.service.as_str()) {
        return GitProxyError::NotFound.into_response();
    }
    let upstream = match resolve(&proxy, &repository_id, &headers).await {
        Ok(upstream) => upstream,
        Err(response) => return response,
    };
    let url = format!("{}/info/refs?service={}", upstream.base_url, query.service);
    forward(
        &proxy,
        upstream,
        reqwest::Method::GET,
        url,
        &headers,
        Body::empty(),
    )
    .await
}

/// `POST /git/{repository_id}/git-upload-pack` — the fetch negotiation and the
/// packfile that follows it.
pub async fn upload_pack(
    State(proxy): State<Arc<GitProxy>>,
    Path(repository_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    service_rpc(proxy, repository_id, "git-upload-pack", headers, body).await
}

/// `POST /git/{repository_id}/git-receive-pack` — the push. The resolved
/// credential must carry `write:repository`, or the host rejects it here.
pub async fn receive_pack(
    State(proxy): State<Arc<GitProxy>>,
    Path(repository_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    service_rpc(proxy, repository_id, "git-receive-pack", headers, body).await
}

async fn service_rpc(
    proxy: Arc<GitProxy>,
    repository_id: String,
    service: &str,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let upstream = match resolve(&proxy, &repository_id, &headers).await {
        Ok(upstream) => upstream,
        Err(response) => return response,
    };
    let url = format!("{}/{service}", upstream.base_url);
    forward(&proxy, upstream, reqwest::Method::POST, url, &headers, body).await
}

async fn resolve(
    proxy: &GitProxy,
    repository_id: &str,
    headers: &HeaderMap,
) -> Result<Upstream, Response> {
    let token = basic_password(headers).ok_or_else(|| {
        GitProxyError::Unauthorized("No task token was presented".to_string()).into_response()
    })?;
    let upstream = proxy
        .resolve(repository_id, &token)
        .await
        .map_err(IntoResponse::into_response)?;
    info!(
        repository_id,
        host = %upstream.host,
        "Proxying a git request"
    );
    Ok(upstream)
}

async fn forward(
    proxy: &GitProxy,
    upstream: Upstream,
    method: reqwest::Method,
    url: String,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    // `x-access-token` is the username every git host ignores when the password
    // is a token; the token itself is what authenticates.
    let mut request = proxy
        .http
        .request(method, &url)
        .basic_auth("x-access-token", Some(&upstream.token))
        .body(reqwest::Body::wrap_stream(
            body.into_data_stream().map_err(std::io::Error::other),
        ));
    for name in FORWARDED_REQUEST_HEADERS {
        if let Some(value) = headers.get(&name) {
            request = request.header(name, value);
        }
    }
    // Git's own capability header. Without it a v2 client's request is answered
    // in v0, and the negotiation it already started fails.
    if let Some(value) = headers.get("git-protocol") {
        request = request.header("git-protocol", value);
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            warn!(%error, host = %upstream.host, "A proxied git request did not reach its host");
            return GitProxyError::Upstream(format!("{} is unreachable", upstream.host))
                .into_response();
        }
    };

    let status = StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut out = Response::builder().status(status);
    for name in FORWARDED_RESPONSE_HEADERS {
        if let Some(value) = response.headers().get(&name) {
            if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) {
                out = out.header(name, value);
            }
        }
    }
    out.body(Body::from_stream(response.bytes_stream()))
        .unwrap_or_else(|error| {
            warn!(%error, "Failed to build a proxied git response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

/// The password half of an HTTP Basic header, which is where git puts a token.
///
/// The username is ignored: `moatless-git-cred.sh` answers `x-access-token`,
/// and a person retrying a URL by hand types whatever they like.
fn basic_password(headers: &HeaderMap) -> Option<String> {
    use base64::Engine as _;

    let encoded = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    Some(decoded.split_once(':')?.1.to_string())
}

impl IntoResponse for GitProxyError {
    fn into_response(self) -> Response {
        let status = match self {
            // `WWW-Authenticate` is what makes git ask its credential helper
            // again rather than give up, so a rotated token is picked up on the
            // retry.
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        let mut response = (status, self.to_string()).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"moatless\""),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn a_basic_header_yields_its_password_whatever_the_username() {
        let mut headers = HeaderMap::new();
        let encoded = base64::engine::general_purpose::STANDARD.encode("anyone:task-jwt");
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {encoded}")).unwrap(),
        );
        assert_eq!(basic_password(&headers).as_deref(), Some("task-jwt"));
    }

    #[test]
    fn a_missing_or_unparsable_header_yields_no_password() {
        assert_eq!(basic_password(&HeaderMap::new()), None);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer task-jwt"),
        );
        assert_eq!(basic_password(&headers), None);
    }
}
