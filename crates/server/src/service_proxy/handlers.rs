//! The one service proxy route.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use moatless_core::proxy_rules::{Caller, NormalPath, Verdict};
use moatless_core::service_proxies::{Injection, ServiceProxy};
use moatless_types::http::RelayedUpstreamFailure;
use tracing::{info, warn};

use super::service::{ServiceProxyError, ServiceProxyGateway};
use crate::proxy::caller::{CallerError, SandboxScope};
use crate::proxy::forward::{bearer_token, forwards, stream_back};

/// `ANY /proxy/{service}/{path}`: one request, checked against the service's
/// rules and forwarded to its upstream with its credential.
pub async fn proxy(
    State(gateway): State<Arc<ServiceProxyGateway>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    match handle(&gateway, method, &uri, &headers, body).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn handle(
    gateway: &ServiceProxyGateway,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, ServiceProxyError> {
    let (name, path_and_query) = split(uri);
    let scope = gateway
        .callers
        .authenticate(bearer_token(headers))
        .await
        .map_err(|e| match e {
            CallerError::Unauthorized(_) | CallerError::SandboxGone => {
                ServiceProxyError::Unauthorized(e.to_string())
            }
            CallerError::Upstream(message) => ServiceProxyError::Upstream(message),
        })?;
    forward(
        gateway,
        &scope,
        &name,
        method,
        &path_and_query,
        headers,
        body,
    )
    .await
}

/// Forward one request for `scope`, which [`proxy`] has authenticated.
pub(super) async fn forward(
    gateway: &ServiceProxyGateway,
    scope: &SandboxScope,
    name: &str,
    method: Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, ServiceProxyError> {
    let service = gateway
        .access
        .services()
        .get(name)
        .ok_or_else(|| ServiceProxyError::UnknownService(name.to_string()))?;
    gateway.check_enabled(scope, service).await?;

    // The rules see the decoded path while the upstream gets the raw one,
    // deliberately: an encoded `/` or `.` decodes into something a rule can
    // refuse, and re-encoding could change what the upstream reads.
    let path = NormalPath::parse(path_and_query)
        .map_err(|e| ServiceProxyError::BadPath(format!("Refused {path_and_query}: {e}")))?;
    let body = axum::body::to_bytes(body, service.limits.request_body_bytes)
        .await
        .map_err(|_| ServiceProxyError::TooLarge(service.limits.request_body_bytes))?;
    let rule = match service.rules.check(
        method.as_str(),
        &path,
        Caller {
            service_account: scope.service_account.as_deref(),
            user: Some(&scope.user),
        },
        Some(&body),
    ) {
        Verdict::Allow(rule) => rule,
        verdict => {
            let reason = match verdict {
                Verdict::BodyRefused(rule) => format!("body_rule:{rule}"),
                _ => "no_rule".to_string(),
            };
            warn!(
                proxy = "service",
                service = %service.name,
                sandbox_id = %scope.sandbox,
                workspace_id = %scope.workspace,
                %method,
                path = path.as_str(),
                reason,
                enforced = true,
                "Proxy request outside its policy"
            );
            return Err(ServiceProxyError::Refused(format!(
                "The {} service proxy allows no {method} {}: its rules in SERVICE_PROXIES_JSON \
                 list what it forwards",
                service.name,
                path.as_str()
            )));
        }
    };

    let secret = gateway.credential(scope, service).await?;
    let client = gateway
        .client(&service.name)
        .ok_or_else(|| ServiceProxyError::Upstream("No client for this service".to_string()))?;
    let url = upstream_url(service, path_and_query, &secret);
    let mut request = client.request(method.clone(), &url).body(body);
    for (header_name, value) in headers {
        if forwards(header_name) && !carries_credential(header_name, service) {
            request = request.header(header_name, value);
        }
    }
    request = inject(request, service, &secret);
    let response = request.send().await.map_err(|error| {
        warn!(%error, service = %service.name, "A service proxy request did not reach its upstream");
        ServiceProxyError::Upstream(format!("{} is unreachable", service.name))
    })?;
    log_forwarded(
        scope,
        service,
        &method,
        &path,
        &rule,
        response.status().as_u16(),
    );
    let mut relayed = stream_back(response, "service");
    if relayed.status().is_server_error() {
        relayed.extensions_mut().insert(RelayedUpstreamFailure);
    }
    Ok(relayed)
}

fn log_forwarded(
    scope: &SandboxScope,
    service: &ServiceProxy,
    method: &Method,
    path: &NormalPath,
    rule: &str,
    status: u16,
) {
    info!(
        proxy = "service",
        service = %service.name,
        sandbox_id = %scope.sandbox,
        workspace_id = %scope.workspace,
        %method,
        path = path.as_str(),
        rule,
        status,
        "Proxied a service request"
    );
}

/// The service a `/proxy/{service}/…` URI names, and the path and query to
/// request from its upstream.
fn split(uri: &Uri) -> (String, String) {
    let rest = uri.path().strip_prefix("/proxy/").unwrap_or_default();
    let (name, path) = rest.split_once('/').unwrap_or((rest, ""));
    let query = uri.query().map_or_default(|q| format!("?{q}"));
    (name.to_string(), format!("/{path}{query}"))
}

/// The upstream URL, with a `query`-style credential replacing whatever value
/// of that parameter the caller sent.
fn upstream_url(service: &ServiceProxy, path_and_query: &str, secret: &str) -> String {
    let joined = format!("{}{path_and_query}", service.upstream);
    let Injection::Query { param } = &service.credential.inject else {
        return joined;
    };
    let Ok(mut url) = reqwest::Url::parse(&joined) else {
        return joined;
    };
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != param.as_str())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(kept)
        .append_pair(param, secret);
    url.to_string()
}

/// Headers that carry the pod's task token or placeholder, its session, or
/// the slot the credential is injected into; none goes upstream.
fn carries_credential(name: &HeaderName, service: &ServiceProxy) -> bool {
    if name == header::AUTHORIZATION || name == header::COOKIE {
        return true;
    }
    if name.as_str().starts_with("x-forwarded-") || name.as_str() == "forwarded" {
        return true;
    }
    matches!(&service.credential.inject, Injection::Header { name: injected, .. }
        if injected.eq_ignore_ascii_case(name.as_str()))
}

fn inject(
    request: reqwest::RequestBuilder,
    service: &ServiceProxy,
    secret: &str,
) -> reqwest::RequestBuilder {
    match &service.credential.inject {
        Injection::Bearer => request.bearer_auth(secret),
        Injection::Header { name, prefix } => {
            request.header(name.as_str(), format!("{prefix}{secret}"))
        }
        Injection::Basic { username } => request.basic_auth(username, Some(secret)),
        Injection::Query { .. } => request,
    }
}

impl IntoResponse for ServiceProxyError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::UnknownService(_) => StatusCode::NOT_FOUND,
            Self::NotEnabled(_) | Self::Refused(_) => StatusCode::FORBIDDEN,
            Self::BadPath(_) => StatusCode::BAD_REQUEST,
            Self::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::NoCredential(_) => StatusCode::FAILED_DEPENDENCY,
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
