//! Application-level auth middleware.
//!
//! Validates session cookies (JWT) and API keys directly, independent of any
//! ingress-layer ForwardAuth. Strips inbound `X-User-*` headers to prevent
//! spoofing, then injects validated headers for downstream extractors.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use tracing::{debug, warn};

use super::api_key::validate_key_cached;
use super::api_keys::ApiKeyRepository;
use super::cache::AuthCache;
use super::cookie::get_all_cookie_values;
use super::identity::AuthIdentity;
use super::jwt;
use super::users::UserRepository;
use crate::config::Config;
use crate::error::ErrorBody;

/// Shared state for the auth middleware layer.
#[derive(Clone)]
pub struct AuthMiddlewareState {
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

    // Webhook and slash-command endpoints use provider-specific verification (HMAC, secret token, etc.)
    // Match /api/v1/integrations/{provider}/webhook,
    // /api/v1/integrations/webhook/{trigger_id}, and
    // /api/v1/integrations/{provider}/slash.
    if let Some(rest) = path.strip_prefix("/api/v1/integrations/") {
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() >= 2
            && (segments[1] == "webhook" || segments[1] == "slash" || segments[0] == "webhook")
        {
            return true;
        }
    }

    // Public endpoints
    matches!(
        path,
        "/api/v1/feature-flags"
            | "/api/v1/integrations/providers"
            | "/health"
            | "/healthz"
            | "/ready"
            | "/openapi.json"
    )
}

/// Strip all inbound `X-User-*`, `X-Group-Id`, and `X-Team-Ids` headers to prevent spoofing.
fn strip_user_headers(req: &mut Request<Body>) {
    let headers = req.headers_mut();
    headers.remove("x-user-id");
    headers.remove("x-user-login");
    headers.remove("x-user-role");
    headers.remove("x-user-provider");
    headers.remove("x-group-id");
    headers.remove("x-team-ids");
}

/// Axum middleware function that validates auth and injects user headers.
pub async fn auth_middleware(mut req: Request<Body>, next: Next) -> Response {
    let state = req.extensions().get::<AuthMiddlewareState>().cloned();

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
                    req.extensions_mut().insert(AuthIdentity {
                        user_id: payload.user_id,
                        login: payload.user_login,
                        role: payload.role,
                        provider: payload.provider,
                        group_id: payload.group_id,
                        team_ids: payload.team_ids,
                    });
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

    // Try session cookies (JWT)
    let Some(jwt_secret) = jwt::get_jwt_secret(&state.config) else {
        warn!(path = %path, "Auth middleware: no JWT secret configured");
        return unauthorized_response();
    };

    let tokens = get_all_cookie_values(req.headers(), &state.config.auth.session_cookie_name);

    for token in &tokens {
        match jwt::verify_session_jwt(token, jwt_secret) {
            Ok(claims) => {
                debug!(user_id = %claims.sub, path = %path, "Auth middleware: session JWT valid");
                let role = claims.user_role();
                req.extensions_mut().insert(AuthIdentity {
                    user_id: claims.sub,
                    login: claims.login,
                    role,
                    provider: claims.provider,
                    group_id: claims.group_id,
                    team_ids: claims.team_ids,
                });
                return next.run(req).await;
            }
            Err(_) => continue,
        }
    }

    // No valid auth found
    if tokens.is_empty() {
        debug!(path = %path, "Auth middleware: no credentials provided");
    } else {
        warn!(path = %path, count = tokens.len(), "Auth middleware: all session cookies invalid/expired");
    }
    unauthorized_response()
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
        assert!(is_exempt("/api/v1/integrations/slack/webhook"));
        assert!(is_exempt("/api/v1/integrations/slack/webhook/"));
        assert!(is_exempt("/api/v1/integrations/telegram/webhook"));
        assert!(is_exempt("/api/v1/integrations/github/webhook"));
        assert!(is_exempt("/api/v1/integrations/linear/webhook"));
        assert!(is_exempt("/api/v1/integrations/webhook/trigger-123"));
        assert!(is_exempt("/api/v1/integrations/slack/slash"));
        // Non-webhook integration routes should NOT be exempt.
        assert!(!is_exempt("/api/v1/integrations/connections"));
        assert!(!is_exempt("/api/v1/triggers"));
    }

    #[test]
    fn exempt_public_routes() {
        assert!(is_exempt("/api/v1/feature-flags"));
        assert!(is_exempt("/api/v1/integrations/providers"));
        assert!(is_exempt("/health"));
        assert!(is_exempt("/healthz"));
        assert!(is_exempt("/ready"));
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
