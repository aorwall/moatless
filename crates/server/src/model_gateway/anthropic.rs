//! The Anthropic Messages API, served with the task's own Anthropic
//! credential or the deployment's hub.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use moatless_core::secret_keys::{
    ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN, ANTHROPIC_BASE_URL, CLAUDE_CODE_OAUTH_TOKEN,
};
use tracing::{info, warn};

use super::service::{GatewayError, ModelGateway};
use super::stream;
use super::stream::presented_token;

/// The beta a Claude OAuth token is accepted under on the Messages API.
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// How the upstream is authenticated.
#[derive(Debug, PartialEq, Eq)]
enum Credential {
    /// `x-api-key`.
    ApiKey(String),
    /// `Authorization: Bearer`, as a hub takes its key.
    Bearer(String),
    /// A Claude subscription's OAuth token: a bearer token under [`OAUTH_BETA`].
    OAuth(String),
}

/// Where a request goes, and with what.
#[derive(Debug, PartialEq, Eq)]
struct Upstream {
    /// Without a trailing `/`.
    base: String,
    credential: Credential,
}

/// Which upstream `credentials` resolve to, in the order the Claude CLI picks
/// its own: an auth token, an API key, the OAuth token, then the hub.
///
/// An `ANTHROPIC_BASE_URL` is honoured only when it names Anthropic or the
/// hub: any user can set one as a secret, and the backend would otherwise
/// fetch any address it can reach with the response going back to the pod.
fn resolve_upstream(
    credentials: &HashMap<String, String>,
    anthropic_base: &str,
    hub_url: Option<&str>,
    hub_key: Option<&str>,
) -> Result<Upstream, GatewayError> {
    let hub_url = hub_url.map(|url| url.trim_end_matches('/'));
    let base = match credentials
        .get(ANTHROPIC_BASE_URL)
        .map(|url| url.trim_end_matches('/'))
    {
        None | Some("") => anthropic_base,
        Some(url) if url == anthropic_base || Some(url) == hub_url => url,
        Some(url) => {
            return Err(GatewayError::NoCredential(format!(
                "ANTHROPIC_BASE_URL {url} is neither Anthropic's API nor this deployment's hub, \
                 so the model gateway does not call it"
            )));
        }
    };
    let credential = credentials
        .get(ANTHROPIC_AUTH_TOKEN)
        .map(|token| Credential::Bearer(token.clone()))
        .or_else(|| {
            credentials
                .get(ANTHROPIC_API_KEY)
                .map(|key| Credential::ApiKey(key.clone()))
        });
    if let Some(credential) = credential {
        return Ok(Upstream {
            base: base.to_string(),
            credential,
        });
    }
    // A hub named with no key of the task's own takes the deployment's key,
    // never the person's OAuth token.
    if Some(base) != hub_url {
        if let Some(token) = credentials.get(CLAUDE_CODE_OAUTH_TOKEN) {
            return Ok(Upstream {
                base: base.to_string(),
                credential: Credential::OAuth(token.clone()),
            });
        }
    }
    match hub_url.zip(hub_key) {
        Some((url, key)) => Ok(Upstream {
            base: url.to_string(),
            credential: Credential::Bearer(key.to_string()),
        }),
        None => Err(GatewayError::NoCredential(
            "This task has no Claude credential: connect Claude or add ANTHROPIC_API_KEY in \
             Settings \u{2192} Secrets"
                .to_string(),
        )),
    }
}

/// `POST /llm/anthropic/v1/messages`.
pub async fn messages(
    State(gateway): State<Arc<ModelGateway>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    forward(&gateway, "/v1/messages", &uri, &headers, body).await
}

/// `POST /llm/anthropic/v1/messages/count_tokens`.
pub async fn count_tokens(
    State(gateway): State<Arc<ModelGateway>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    forward(&gateway, "/v1/messages/count_tokens", &uri, &headers, body).await
}

async fn forward(
    gateway: &ModelGateway,
    path: &str,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let scope = match gateway.authenticate(presented_token(headers)).await {
        Ok(scope) => scope,
        Err(e) => return error_response(&e),
    };
    let mode = match super::auth_mode::AuthMode::parse(headers) {
        Ok(mode) => mode,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let mut credentials = match gateway.anthropic_credentials(&scope, mode).await {
        Ok(credentials) => credentials,
        Err(e) => return error_response(&e),
    };
    mode.filter(&mut credentials);
    let upstream = match resolve_upstream(
        &credentials,
        &gateway.bases.anthropic,
        gateway.config.hub_url.as_deref(),
        gateway.config.hub_key.as_deref(),
    ) {
        Ok(upstream) => upstream,
        Err(e) => return error_response(&e),
    };

    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let url = format!("{}{path}{query}", upstream.base);
    let (request, kind) =
        authorized_request(&gateway.http, &url, headers, body, upstream.credential);
    match stream::relay(request).await {
        Ok(response) => {
            info!(
                sandbox_id = %scope.sandbox,
                path,
                credential = kind,
                status = response.status().as_u16(),
                "Proxied a model request"
            );
            response
        }
        Err(error) => {
            warn!(%error, sandbox_id = %scope.sandbox, path, "A model request did not reach its upstream");
            error_response(&GatewayError::Upstream(
                "The model provider is unreachable".to_string(),
            ))
        }
    }
}

/// The upstream request carrying `credential`, and the credential's kind for
/// the log.
fn authorized_request(
    http: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    body: Body,
    credential: Credential,
) -> (reqwest::RequestBuilder, &'static str) {
    match credential {
        Credential::ApiKey(key) => (
            stream::upstream_request(http, Method::POST, url, headers, body)
                .header("x-api-key", key),
            "api_key",
        ),
        Credential::Bearer(token) => (
            stream::upstream_request(http, Method::POST, url, headers, body).bearer_auth(token),
            "bearer",
        ),
        Credential::OAuth(token) => {
            // Replaced rather than appended: a second `anthropic-beta` would
            // send every caller beta twice.
            let mut headers = headers.clone();
            if let Ok(betas) = HeaderValue::from_str(&with_oauth_beta(&headers)) {
                headers.insert("anthropic-beta", betas);
            }
            (
                stream::upstream_request(http, Method::POST, url, &headers, body)
                    .bearer_auth(token),
                "oauth",
            )
        }
    }
}

/// The caller's `anthropic-beta` list with [`OAUTH_BETA`] added once.
fn with_oauth_beta(headers: &HeaderMap) -> String {
    let mut betas: Vec<&str> = headers
        .get_all("anthropic-beta")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|beta| !beta.is_empty())
        .collect();
    if !betas.contains(&OAUTH_BETA) {
        betas.push(OAUTH_BETA);
    }
    betas.join(",")
}

/// An Anthropic-shaped error, which the Claude CLI prints and classifies.
fn error_response(error: &GatewayError) -> Response {
    let (status, kind) = match error {
        GatewayError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "authentication_error"),
        GatewayError::NoCredential(_) => (StatusCode::FORBIDDEN, "permission_error"),
        GatewayError::Upstream(_) => (StatusCode::BAD_GATEWAY, "api_error"),
    };
    (
        status,
        axum::Json(serde_json::json!({
            "type": "error",
            "error": { "type": kind, "message": error.to_string() },
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
