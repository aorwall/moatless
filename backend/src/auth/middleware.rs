//! Application-level auth middleware.
//!
//! Validates session cookies and API keys directly, independent of any
//! ingress-layer ForwardAuth. Strips inbound `X-User-*` headers to prevent
//! spoofing, then injects validated headers for downstream extractors.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use tracing::{debug, warn};

use super::api_key::validate_key_cached;
use super::api_keys::ApiKeyRepository;
use super::cache::AuthCache;
use super::cookie::get_all_cookie_values;
use super::session::validate_session_cached;
use super::sessions::SessionRepository;
use super::users::UserRepository;
use crate::config::Config;
use crate::sandbox::schemas::ErrorBody;

/// Shared state for the auth middleware layer.
#[derive(Clone)]
pub struct AuthMiddlewareState {
    pub sessions: Arc<dyn SessionRepository>,
    pub api_keys: Arc<dyn ApiKeyRepository>,
    pub users: Arc<dyn UserRepository>,
    pub cache: AuthCache,
    pub config: Arc<Config>,
}

/// Paths exempt from auth middleware validation.
fn is_exempt(path: &str) -> bool {
    // Auth routes handle their own authentication
    if path.starts_with("/api/v1/auth/") || path == "/api/v1/auth" {
        return true;
    }

    // Sandbox routes use JWT-based task tokens
    if path.starts_with("/api/sandbox/") {
        return true;
    }

    // Webhook endpoints use HMAC verification
    if path == "/api/v1/channels/slack/webhook"
        || path.starts_with("/api/v1/channels/slack/webhook/")
        || path.starts_with("/api/v1/channels/webhook/")
    {
        return true;
    }

    // Public endpoints
    matches!(
        path,
        "/api/v1/feature-flags" | "/health" | "/healthz" | "/openapi.json"
    )
}

/// Strip all inbound `X-User-*` and `X-Group-Id` headers to prevent spoofing.
fn strip_user_headers(req: &mut Request<Body>) {
    let headers = req.headers_mut();
    headers.remove("x-user-id");
    headers.remove("x-user-login");
    headers.remove("x-user-role");
    headers.remove("x-user-provider");
    headers.remove("x-group-id");
}

/// Axum middleware function that validates auth and injects user headers.
pub async fn auth_middleware(mut req: Request<Body>, next: Next) -> Response {
    let state = req
        .extensions()
        .get::<AuthMiddlewareState>()
        .cloned();

    let Some(state) = state else {
        // No middleware state — pass through (shouldn't happen in practice)
        return next.run(req).await;
    };

    // Dev mode: no auth provider configured — pass through
    if state.config.auth.provider.is_none() {
        return next.run(req).await;
    }

    let path = req.uri().path().to_string();

    // Exempt paths bypass auth
    if is_exempt(&path) {
        return next.run(req).await;
    }

    // Strip spoofable headers before any validation
    strip_user_headers(&mut req);

    // Try API key (Authorization: Bearer mvk_xxx)
    if let Some(auth_header) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if auth_header.starts_with("Bearer mvk_") {
            let key = &auth_header[7..]; // strip "Bearer "
            match validate_key_cached(
                state.api_keys.clone(),
                state.users.as_ref(),
                key,
                &state.cache,
            )
            .await
            {
                Ok(Some(payload)) => {
                    debug!(user_id = %payload.user_id, path = %path, "Auth middleware: API key valid");
                    inject_user_headers(&mut req, &payload);
                    return next.run(req).await;
                }
                Ok(None) => {
                    warn!(path = %path, "Auth middleware: invalid API key");
                    return unauthorized_response();
                }
                Err(e) => {
                    warn!(error = %e, path = %path, "Auth middleware: API key validation error");
                    return unauthorized_response();
                }
            }
        }
    }

    // Try session cookies
    let session_ids =
        get_all_cookie_values(req.headers(), &state.config.auth.session_cookie_name);

    for session_id in &session_ids {
        match validate_session_cached(state.sessions.as_ref(), session_id, &state.cache).await {
            Ok(Some(payload)) => {
                debug!(user_id = %payload.user_id, path = %path, "Auth middleware: session valid");
                inject_user_headers(&mut req, &payload);
                return next.run(req).await;
            }
            Ok(None) => continue,
            Err(e) => {
                warn!(error = %e, path = %path, "Auth middleware: session validation error");
                return unauthorized_response();
            }
        }
    }

    // No valid auth found
    if session_ids.is_empty() {
        debug!(path = %path, "Auth middleware: no credentials provided");
    } else {
        warn!(path = %path, count = session_ids.len(), "Auth middleware: all session cookies invalid/expired");
    }
    unauthorized_response()
}

/// Inject validated user headers into the request for downstream extractors.
fn inject_user_headers(req: &mut Request<Body>, payload: &super::cache::VerifyPayload) {
    let headers = req.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&payload.user_id) {
        headers.insert("x-user-id", v);
    }
    if let Ok(v) = HeaderValue::from_str(&payload.user_login) {
        headers.insert("x-user-login", v);
    }
    if let Ok(v) = HeaderValue::from_str(payload.role.as_str()) {
        headers.insert("x-user-role", v);
    }
    if let Ok(v) = HeaderValue::from_str(&payload.provider) {
        headers.insert("x-user-provider", v);
    }
    if let Some(ref gid) = payload.group_id {
        if let Ok(v) = HeaderValue::from_str(gid) {
            headers.insert("x-group-id", v);
        }
    }
}

/// Build a 401 JSON response.
fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody {
            error: "Authentication required".to_string(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exempt_auth_routes() {
        assert!(is_exempt("/api/v1/auth/me"));
        assert!(is_exempt("/api/v1/auth/verify"));
        assert!(is_exempt("/api/v1/auth/login"));
        assert!(is_exempt("/api/v1/auth/github"));
        assert!(is_exempt("/api/v1/auth/github/callback"));
        assert!(is_exempt("/api/v1/auth/auth0"));
        assert!(is_exempt("/api/v1/auth/auth0/callback"));
        assert!(is_exempt("/api/v1/auth/logout"));
        assert!(is_exempt("/api/v1/auth/api-keys"));
        assert!(is_exempt("/api/v1/auth/api-key/session"));
    }

    #[test]
    fn exempt_sandbox_routes() {
        assert!(is_exempt("/api/sandbox/v1/tasks/abc/provision"));
        assert!(is_exempt("/api/sandbox/v1/tasks/abc/status"));
        assert!(is_exempt("/api/sandbox/v1/tasks/abc/stop"));
    }

    #[test]
    fn exempt_webhook_routes() {
        assert!(is_exempt("/api/v1/channels/slack/webhook"));
        assert!(is_exempt("/api/v1/channels/slack/webhook/"));
        assert!(is_exempt("/api/v1/channels/webhook/some-provider"));
    }

    #[test]
    fn exempt_public_routes() {
        assert!(is_exempt("/api/v1/feature-flags"));
        assert!(is_exempt("/health"));
        assert!(is_exempt("/healthz"));
        assert!(is_exempt("/openapi.json"));
    }

    #[test]
    fn non_exempt_domain_routes() {
        assert!(!is_exempt("/api/v1/tasks"));
        assert!(!is_exempt("/api/v1/tasks/abc"));
        assert!(!is_exempt("/api/v1/events/stream"));
        assert!(!is_exempt("/api/v1/users"));
        assert!(!is_exempt("/api/v1/repositories"));
        assert!(!is_exempt("/api/v1/messages"));
        assert!(!is_exempt("/api/v1/commands"));
    }
}
