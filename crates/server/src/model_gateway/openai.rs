//! The OpenAI Responses API, which a pod's Codex reaches its model through,
//! served with the task's own OpenAI credential or the deployment's hub.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use moatless_core::secret_keys::{CODEX_AUTH_JSON, OPENAI_API_KEY, OPENAI_BASE_URL};
use tracing::{info, warn};

use super::service::{GatewayError, ModelGateway, ProviderBases};
use super::stream;
use super::stream::presented_token;

/// How the upstream is authenticated.
#[derive(Debug, PartialEq, Eq)]
enum Credential {
    /// `Authorization: Bearer`, as an API key or a hub key is presented.
    Bearer(String),
    /// A ChatGPT login: its access token, and the account Codex would name.
    ChatGpt {
        access_token: String,
        account_id: Option<String>,
    },
}

/// Where a request goes, and with what.
#[derive(Debug, PartialEq, Eq)]
struct Upstream {
    /// Without a trailing `/`; the Codex path follows it.
    base: String,
    credential: Credential,
}

/// Which upstream `credentials` resolve to: an `OPENAI_BASE_URL`, the Codex
/// login, an API key, then the hub.
///
/// An `OPENAI_BASE_URL` must name OpenAI's API or the hub's `/v1`, for the
/// reason [`super::anthropic`] allow-lists `ANTHROPIC_BASE_URL`.
fn resolve_upstream(
    credentials: &HashMap<String, String>,
    bases: &ProviderBases,
    hub_url: Option<&str>,
    hub_key: Option<&str>,
) -> Result<Upstream, GatewayError> {
    let present = |key: &str| {
        credentials
            .get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    };
    let hub_v1 = hub_url.map(|url| format!("{}/v1", url.trim_end_matches('/')));
    let api_key = present(OPENAI_API_KEY);
    match present(OPENAI_BASE_URL).map(|url| url.trim_end_matches('/')) {
        None => {}
        Some(url) if url == bases.openai => {}
        // A hub named with no key of the task's own takes the deployment's
        // key, never the person's Codex login.
        Some(url) if hub_v1.as_deref() == Some(url) => {
            return match api_key.or(hub_key) {
                Some(key) => Ok(Upstream {
                    base: url.to_string(),
                    credential: Credential::Bearer(key.to_string()),
                }),
                None => Err(GatewayError::NoCredential(format!(
                    "{OPENAI_BASE_URL} names this deployment's hub, but neither the task nor \
                     the deployment holds a key for it"
                ))),
            };
        }
        Some(url) => {
            return Err(GatewayError::NoCredential(format!(
                "{OPENAI_BASE_URL} {url} is neither OpenAI's API nor this deployment's hub, so \
                 the model gateway does not call it"
            )));
        }
    }
    if let Some(upstream) = present(CODEX_AUTH_JSON).and_then(|blob| codex_login(blob, bases)) {
        return Ok(upstream);
    }
    if let Some(key) = api_key {
        return Ok(Upstream {
            base: bases.openai.clone(),
            credential: Credential::Bearer(key.to_string()),
        });
    }
    match hub_v1.zip(hub_key) {
        Some((base, key)) => Ok(Upstream {
            base,
            credential: Credential::Bearer(key.to_string()),
        }),
        None => Err(GatewayError::NoCredential(format!(
            "This task has no OpenAI credential: connect Codex or add {OPENAI_API_KEY} in \
             Settings \u{2192} Secrets"
        ))),
    }
}

/// Whether [`resolve_upstream`] reaches the Codex login for `credentials`:
/// it does unless an `OPENAI_BASE_URL` other than OpenAI's API is set.
pub(super) fn reads_codex_login(
    credentials: &HashMap<String, String>,
    bases: &ProviderBases,
) -> bool {
    credentials
        .get(OPENAI_BASE_URL)
        .map(|url| url.trim().trim_end_matches('/'))
        .is_none_or(|url| url.is_empty() || url == bases.openai)
}

/// A Codex `auth.json`'s upstream, read the way the Codex CLI reads it: its
/// API key in `apikey` mode or when it has no `auth_mode` and holds one, else
/// its ChatGPT tokens.
fn codex_login(blob: &str, bases: &ProviderBases) -> Option<Upstream> {
    let auth: serde_json::Value = match serde_json::from_str(blob) {
        Ok(auth) => auth,
        Err(error) => {
            warn!(%error, "The task's Codex auth.json does not parse");
            return None;
        }
    };
    let text = |value: Option<&serde_json::Value>| {
        value
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let mode = auth.get("auth_mode").and_then(serde_json::Value::as_str);
    if let Some(key) =
        text(auth.get(OPENAI_API_KEY)).filter(|_| matches!(mode, None | Some("apikey")))
    {
        return Some(Upstream {
            base: bases.openai.clone(),
            credential: Credential::Bearer(key),
        });
    }
    let tokens = auth.get("tokens");
    Some(Upstream {
        base: bases.chatgpt.clone(),
        credential: Credential::ChatGpt {
            access_token: text(tokens.and_then(|t| t.get("access_token")))?,
            account_id: text(tokens.and_then(|t| t.get("account_id"))),
        },
    })
}

/// `POST /llm/openai/v1/responses`.
pub async fn responses(
    State(gateway): State<Arc<ModelGateway>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    forward(&gateway, Method::POST, "/responses", &uri, &headers, body).await
}

/// `POST /llm/openai/v1/responses/compact`, which Codex compacts a long
/// conversation through.
pub async fn compact(
    State(gateway): State<Arc<ModelGateway>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    forward(
        &gateway,
        Method::POST,
        "/responses/compact",
        &uri,
        &headers,
        body,
    )
    .await
}

/// `GET /llm/openai/v1/models`, which Codex reads its model list from.
pub async fn models(
    State(gateway): State<Arc<ModelGateway>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    forward(
        &gateway,
        Method::GET,
        "/models",
        &uri,
        &headers,
        Body::empty(),
    )
    .await
}

async fn forward(
    gateway: &ModelGateway,
    method: Method,
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
    let mut credentials = match gateway.openai_credentials(&scope, mode).await {
        Ok(credentials) => credentials,
        Err(e) => return error_response(&e),
    };
    mode.filter(&mut credentials);
    let upstream = match resolve_upstream(
        &credentials,
        &gateway.bases,
        gateway.config.hub_url.as_deref(),
        gateway.config.hub_key.as_deref(),
    ) {
        Ok(upstream) => upstream,
        Err(e) => return error_response(&e),
    };

    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let url = format!("{}{path}{query}", upstream.base);
    let (request, kind) = authorized_request(
        stream::upstream_request(&gateway.http, method, &url, headers, body),
        upstream.credential,
    );
    match stream::relay(request).await {
        Ok(response) => {
            info!(
                sandbox_id = %scope.sandbox,
                path,
                credential = kind,
                status = response.status().as_u16(),
                "Proxied a model request"
            );
            fail_long_throttle(response, Utc::now())
        }
        Err(error) => {
            warn!(%error, sandbox_id = %scope.sandbox, path, "A model request did not reach its upstream");
            error_response(&GatewayError::Upstream(
                "The model provider is unreachable".to_string(),
            ))
        }
    }
}

/// The longest `Retry-After` Codex is left to wait out. Codex sleeps the full
/// header with its Turn still open, so a hub's days-long credential cooldown
/// froze every Codex task while it showed as running.
const LONGEST_WAITED_RETRY: Duration = Duration::from_secs(60);

/// `response`, unless it is a 429 whose `Retry-After` is past
/// [`LONGEST_WAITED_RETRY`]: that becomes OpenAI's `usage_limit_reached`, which
/// Codex fails the Turn on rather than sleeping, so the harness reports a
/// usage limit and a Loop can fall back to its other agent.
fn fail_long_throttle(response: Response, now: DateTime<Utc>) -> Response {
    if response.status() != StatusCode::TOO_MANY_REQUESTS {
        return response;
    }
    let Some(wait) = retry_after(response.headers(), now) else {
        return response;
    };
    if wait <= LONGEST_WAITED_RETRY {
        return response;
    }
    let wait_secs = i64::try_from(wait.as_secs()).unwrap_or(i64::MAX);
    warn!(
        retry_after_secs = wait_secs,
        "The model provider throttled a request for longer than Codex should wait; failing the Turn as a usage limit"
    );
    let mut out = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(serde_json::json!({
            "error": {
                "type": "usage_limit_reached",
                "message": format!("The model provider asked to retry in {wait_secs} seconds"),
                "resets_at": now.timestamp().saturating_add(wait_secs),
            },
        })),
    )
        .into_response();
    // The provider's own rate-limit readings stay; Codex reports them.
    for (name, value) in response.headers() {
        if name.as_str().starts_with("x-codex-") {
            out.headers_mut().append(name.clone(), value.clone());
        }
    }
    out
}

/// A `Retry-After` header as a wait from `now`, in either of its forms:
/// delay-seconds or an HTTP-date.
fn retry_after(headers: &HeaderMap, now: DateTime<Utc>) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = DateTime::parse_from_rfc2822(value).ok()?;
    (at.with_timezone(&Utc) - now).to_std().ok()
}

/// `request` presenting `credential`, and the kind it logs as.
fn authorized_request(
    request: reqwest::RequestBuilder,
    credential: Credential,
) -> (reqwest::RequestBuilder, &'static str) {
    match credential {
        Credential::Bearer(token) => (request.bearer_auth(token), "bearer"),
        Credential::ChatGpt {
            access_token,
            account_id,
        } => {
            let request = request.bearer_auth(access_token);
            let request = match account_id {
                Some(id) => request.header("chatgpt-account-id", id),
                None => request,
            };
            (request, "chatgpt")
        }
    }
}

/// An OpenAI-shaped error. Codex reruns its provider's auth command after a
/// `401`, so a pod whose token generation was bumped retries with the token
/// file's new one.
fn error_response(error: &GatewayError) -> Response {
    let (status, kind) = match error {
        GatewayError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "invalid_request_error"),
        GatewayError::NoCredential(_) => (StatusCode::FORBIDDEN, "permission_error"),
        GatewayError::Upstream(_) => (StatusCode::BAD_GATEWAY, "server_error"),
    };
    (
        status,
        axum::Json(serde_json::json!({
            "error": { "type": kind, "message": error.to_string() },
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
