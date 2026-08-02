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
use uuid::Uuid;

use super::api_key::validate_key_cached;
use super::api_keys::ApiKeyDao;
use super::cache::AuthCache;
use super::cookie::get_all_cookie_values;
use super::identity::{AuthIdentity, AuthMethod};
use super::jwt;
use super::users::UserDao;
use crate::config::Config;
use crate::error::ErrorBody;

/// The one route that accepts a ticket in its query string.
const WEBSOCKET_PATH: &str = "/ws";

/// The query parameter a WebSocket ticket arrives in. Named by T3 Code's
/// client, which is the only thing that sends one.
const WEBSOCKET_TICKET_PARAM: &str = "wsTicket";

/// Shared state for the auth middleware layer.
#[derive(Clone)]
pub struct AuthMiddlewareState {
    pub api_keys: Arc<dyn ApiKeyDao>,
    pub users: Arc<dyn UserDao>,
    pub cache: AuthCache,
    pub config: Arc<Config>,
}

/// Paths exempt from auth middleware validation.
fn is_exempt(path: &str) -> bool {
    // Auth routes handle their own authentication
    if path.starts_with("/api/v1/auth/") || path == "/api/v1/auth" {
        return true;
    }

    // NOTE: `/api/sandbox/*` is intentionally NOT exempt. These endpoints run
    // through this middleware (session cookie or API key) so that (a) spoofable
    // `X-User-*` headers are stripped and (b) an `AuthIdentity` is injected for
    // the per-task authorization the sandbox handlers now enforce. Every real
    // caller (frontend, CLI) already carries a session cookie or API key.

    // Webhook and slash-command endpoints use adapter-specific verification (HMAC, secret token, etc.)
    // Match /api/v1/adapters/{adapter}/webhook,
    // /api/v1/adapters/webhook/{loop_id}, and
    // /api/v1/adapters/{adapter}/slash.
    if let Some(rest) = path.strip_prefix("/api/v1/adapters/") {
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() >= 2
            && (segments[1] == "webhook" || segments[1] == "slash" || segments[0] == "webhook")
        {
            return true;
        }
    }

    // Legacy webhook callback aliases. These are the only `/channels/*` routes.
    if let Some(rest) = path.strip_prefix("/api/v1/channels/") {
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() >= 2 && (segments[1] == "webhook" || segments[0] == "webhook") {
            return true;
        }
    }

    // Public endpoints
    matches!(
        path,
        "/.well-known/t3/environment"
            // Bootstrap, not public: the pairing code in the body is the
            // credential, and a caller that had another one would not be here.
            // Mounted only when the T3 compatibility routes are (see
            // `moatless_t3code::router_from_env`); a 404 otherwise.
            | "/oauth/token"
            | "/api/v1/feature-flags"
            | "/api/v1/adapters"
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

fn insert_auth_extensions(req: &mut Request<Body>, identity: AuthIdentity, method: AuthMethod) {
    let ui_rpc_role = match identity.role.as_str() {
        "admin" => moatless_t3code::UserRole::Admin,
        _ => moatless_t3code::UserRole::User,
    };
    req.extensions_mut()
        .insert(moatless_t3code::UiRpcAuthIdentity {
            user_id: identity.user_id,
            login: identity.login.clone(),
            role: ui_rpc_role,
            provider: identity.provider.clone(),
            group_id: identity.group_id.clone(),
        });
    req.extensions_mut().insert(match method {
        AuthMethod::SessionCookie => moatless_t3code::UiRpcAuthMethod::SessionCookie,
        AuthMethod::ApiKey => moatless_t3code::UiRpcAuthMethod::ApiKey,
        AuthMethod::WebSocketTicket => moatless_t3code::UiRpcAuthMethod::WebSocketTicket,
    });
    req.extensions_mut().insert(identity);
    req.extensions_mut().insert(method);
}

/// Axum middleware function that validates auth and injects user headers.
pub async fn auth_middleware(mut req: Request<Body>, next: Next) -> Response {
    let state = req.extensions().get::<AuthMiddlewareState>().cloned();

    let Some(state) = state else {
        // No middleware state — pass through (shouldn't happen in practice)
        return next.run(req).await;
    };

    let path = req.uri().path().to_string();

    // Exempt paths bypass auth
    if is_exempt(&path) {
        return next.run(req).await;
    }

    // Strip spoofable headers before any validation
    strip_user_headers(&mut req);

    // A WebSocket upgrade carries no headers a client can choose, so the
    // credential is in the query string instead. Only on this one path, and
    // only a ticket: nothing else is accepted there, and a ticket is accepted
    // nowhere else. A present-but-bad ticket is refused rather than falling
    // through to the cookie, so a client that thought it was authenticating one
    // way is not silently authenticated another.
    if path == WEBSOCKET_PATH {
        if let Some(ticket) = websocket_ticket(&req) {
            let Some(jwt_secret) = jwt::get_jwt_secret(&state.config) else {
                warn!(path = %path, "Auth middleware: no JWT secret configured");
                return unauthorized_response();
            };
            return match jwt::verify_ws_ticket_jwt(&ticket, jwt_secret) {
                Ok(claims) => {
                    debug!(user_id = %claims.sub, path = %path, "Auth middleware: websocket ticket valid");
                    tracing::Span::current().record("user_id", claims.sub.to_string().as_str());
                    let role = claims.user_role();
                    insert_auth_extensions(
                        &mut req,
                        AuthIdentity {
                            user_id: claims.sub,
                            login: claims.login,
                            role,
                            provider: claims.provider,
                            group_id: claims.group_id,
                        },
                        AuthMethod::WebSocketTicket,
                    );
                    next.run(req).await
                }
                Err(error) => {
                    warn!(error = %error, path = %path, "Auth middleware: invalid websocket ticket");
                    unauthorized_response()
                }
            };
        }
    }

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
                    let Ok(user_id) = payload.user_id.parse::<Uuid>() else {
                        warn!(path = %path, user_id = %payload.user_id, "Auth middleware: API key user_id is not a uuid");
                        return unauthorized_response();
                    };
                    debug!(user_id = %user_id, path = %path, "Auth middleware: API key valid");
                    // Stamp user_id onto the surrounding request-trace span so
                    // every downstream log line carries it.
                    tracing::Span::current().record("user_id", user_id.to_string().as_str());
                    insert_auth_extensions(
                        &mut req,
                        AuthIdentity {
                            user_id,
                            login: payload.user_login,
                            role: payload.role,
                            provider: payload.provider,
                            group_id: payload.group_id,
                        },
                        AuthMethod::ApiKey,
                    );
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
                // Stamp user_id onto the surrounding request-trace span so
                // every downstream log line carries it.
                tracing::Span::current().record("user_id", claims.sub.to_string().as_str());
                let role = claims.user_role();
                insert_auth_extensions(
                    &mut req,
                    AuthIdentity {
                        user_id: claims.sub,
                        login: claims.login,
                        role,
                        provider: claims.provider,
                        group_id: claims.group_id,
                    },
                    AuthMethod::SessionCookie,
                );
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

/// The WebSocket ticket this request presents, if it presents one.
fn websocket_ticket(req: &Request<Body>) -> Option<String> {
    req.uri().query().and_then(|query| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            if key != WEBSOCKET_TICKET_PARAM {
                return None;
            }
            let value = urlencoding::decode(value).ok()?;
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        })
    })
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
    fn sandbox_routes_are_not_exempt() {
        // Sandbox routes must run through the middleware so callers are
        // authenticated (cookie/API key) and per-task authorization applies.
        assert!(!is_exempt("/api/sandbox/v1/tasks/abc/provision"));
        assert!(!is_exempt("/api/sandbox/v1/tasks/abc/status"));
        assert!(!is_exempt("/api/sandbox/v1/tasks/abc/stop"));
        assert!(!is_exempt("/api/sandbox/v1/tasks/abc/files/read"));
    }

    #[test]
    fn only_t3_discovery_and_the_pairing_exchange_are_public() {
        assert!(is_exempt("/.well-known/t3/environment"));
        // The exchange carries its own credential in the body.
        assert!(is_exempt("/oauth/token"));
        assert!(!is_exempt("/api/auth/session"));
        assert!(!is_exempt("/api/auth/websocket-ticket"));
        assert!(!is_exempt("/api/orchestration/shell"));
        assert!(!is_exempt("/ws"));
    }

    fn request_to(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[test]
    fn a_ticket_is_read_off_the_query_string() {
        assert_eq!(
            websocket_ticket(&request_to("/ws?wsTicket=abc.def.ghi")).as_deref(),
            Some("abc.def.ghi")
        );
        assert_eq!(
            websocket_ticket(&request_to("/ws?other=1&wsTicket=abc&more=2")).as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn a_request_without_a_ticket_has_none() {
        assert!(websocket_ticket(&request_to("/ws")).is_none());
        assert!(websocket_ticket(&request_to("/ws?wsTicket=")).is_none());
        assert!(websocket_ticket(&request_to("/ws?wsticket=abc")).is_none());
        assert!(websocket_ticket(&request_to("/ws?authToken=abc")).is_none());
    }

    /// Percent-encoded because that is what `URLSearchParams` produces, which is
    /// what builds the URL on the other side.
    #[test]
    fn a_percent_encoded_ticket_is_decoded() {
        assert_eq!(
            websocket_ticket(&request_to("/ws?wsTicket=abc%2Edef")).as_deref(),
            Some("abc.def")
        );
    }

    #[test]
    fn exempt_webhook_routes() {
        assert!(is_exempt("/api/v1/adapters/slack/webhook"));
        assert!(is_exempt("/api/v1/adapters/slack/webhook/"));
        assert!(is_exempt("/api/v1/adapters/telegram/webhook"));
        assert!(is_exempt("/api/v1/adapters/github/webhook"));
        assert!(is_exempt("/api/v1/adapters/linear/webhook"));
        assert!(is_exempt("/api/v1/adapters/webhook/trigger-123"));
        assert!(is_exempt("/api/v1/adapters/slack/slash"));
        assert!(is_exempt("/api/v1/channels/slack/webhook"));
        assert!(is_exempt("/api/v1/channels/webhook/trigger-123"));
        assert!(!is_exempt("/api/v1/channels/slack/slash"));
        // Non-webhook integration routes should NOT be exempt.
        assert!(!is_exempt("/api/v1/adapters/connections"));
        assert!(!is_exempt("/api/v1/loops"));
    }

    #[test]
    fn exempt_public_routes() {
        assert!(is_exempt("/api/v1/feature-flags"));
        assert!(is_exempt("/api/v1/adapters"));
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
