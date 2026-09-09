//! Application-level auth middleware.
//!
//! Validates opaque auth-session cookies and API keys directly, independent of any
//! ingress-layer `ForwardAuth`. Strips inbound `X-User-*` headers to prevent
//! spoofing, then injects validated headers for downstream extractors.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use moatless_types::ui_rpc::{UiRpcAuthIdentity, UiRpcAuthMethod, UserRole};
use tracing::{debug, warn};
use uuid::Uuid;

use super::api_key::validate_key_cached;
use super::api_keys_dao::ApiKeyDao;
use super::cache::AuthCache;
use super::cookie::{CSRF_COOKIE_NAME, get_all_cookie_values, get_cookie_value};
use super::jwt;
use super::sessions as session;
use super::sessions::SessionDao;
use super::users_dao::UserDao;
use moatless_api_schemas::error::ErrorBody;
use moatless_core::config::Config;
use moatless_core::http_auth::UserTokenVerifier;
use moatless_types::auth::{AuthIdentity, AuthMethod};
use moatless_types::user::UserStatus;

/// The one route that accepts a ticket in its query string.
const WEBSOCKET_PATH: &str = "/ws";

/// The query parameter a WebSocket ticket arrives in. Named by T3 Code's
/// client, which is the only thing that sends one.
const WEBSOCKET_TICKET_PARAM: &str = "wsTicket";

/// Shared state for the auth middleware layer.
#[derive(Clone)]
pub struct AuthMiddlewareState {
    pub(crate) api_keys: Arc<dyn ApiKeyDao>,
    pub(crate) users: Arc<dyn UserDao>,
    pub(super) sessions: Arc<dyn SessionDao>,
    pub token_verifier: Arc<dyn UserTokenVerifier>,
    pub cache: AuthCache,
    pub config: Arc<Config>,
}

/// Paths exempt from auth middleware validation.
fn is_exempt(path: &str) -> bool {
    // Login/bootstrap routes carry their own one-time credential. Authenticated
    // auth-management routes intentionally remain behind this middleware.
    if matches!(
        path,
        "/api/v1/auth/mode"
            | "/api/v1/auth/password/login"
            | "/api/v1/auth/login"
            | "/api/v1/auth/cli/login"
            | "/api/v1/auth/github-app"
            | "/api/v1/auth/github-app/callback"
            | "/api/v1/auth/auth0"
            | "/api/v1/auth/auth0/callback"
            | "/api/v1/auth/linear/install"
            | "/api/v1/auth/linear/callback"
            | "/api/v1/auth/token/session"
    ) || (path.starts_with("/api/v1/auth/oidc/")
        && (path.ends_with("/start") || path.ends_with("/callback")))
    {
        return true;
    }

    // NOTE: `/api/sandbox/*` is intentionally NOT exempt. These endpoints run
    // through this middleware (session cookie or API key) so that (a) spoofable
    // `X-User-*` headers are stripped and (b) an `AuthIdentity` is injected for
    // the per-task authorization the sandbox handlers enforce. Every real
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

    // A T3 chat attachment upload. Not public: the path segment after the
    // prefix is a short-lived signed token naming the viewer who minted it and
    // the one attachment it may write, and the route verifies it before
    // reading a byte (`moatless_t3code::attachments`). Exempt because that
    // credential is the point — the client uploads with a bare
    // `xhr.send(file)`, which carries no CSRF header, and a route that fell
    // back to the ambient session cookie would be the CSRF the header exists to
    // stop. Mounted only when the T3 compatibility routes are; a 404 otherwise.
    if path.starts_with("/api/attachments/") {
        return true;
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
        "admin" => UserRole::Admin,
        _ => UserRole::User,
    };
    req.extensions_mut().insert(UiRpcAuthIdentity {
        user_id: identity.user_id,
        login: identity.login.clone(),
        role: ui_rpc_role,
        provider: identity.provider.clone(),
        group_id: identity.group_id.clone(),
    });
    req.extensions_mut().insert(match method {
        AuthMethod::SessionCookie => UiRpcAuthMethod::SessionCookie,
        AuthMethod::ApiKey => UiRpcAuthMethod::ApiKey,
        AuthMethod::WebSocketTicket => UiRpcAuthMethod::WebSocketTicket,
        // A held bearer credential like the API key — never attached ambiently,
        // so the UI-RPC origin defence treats it the same way.
        AuthMethod::UserToken => UiRpcAuthMethod::ApiKey,
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

    // Try the sandbox-issued X-User-Token JWT. A deliberately-held bearer
    // credential, so — like the API key above and unlike the session cookie
    // below — it is CSRF-exempt: it is never attached ambiently by a browser.
    // Tried ahead of the cookie so a sandbox request carrying both authenticates
    // as the agent without hitting the cookie's CSRF gate. A present-but-invalid
    // token (or no verifier configured) falls through to the remaining methods
    // rather than failing the request, so it never narrows what already worked.
    if let Some(token) = req
        .headers()
        .get("x-user-token")
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(verified) = state.token_verifier.verify_user_token(token) {
            match verified.user_id.parse::<Uuid>() {
                Ok(user_id) => match state.users.get_user(&user_id).await {
                    Ok(Some(user)) if user.status == UserStatus::Active => {
                        debug!(user_id = %user_id, path = %path, "Auth middleware: user token valid");
                        tracing::Span::current().record("user_id", user_id.to_string().as_str());
                        insert_auth_extensions(
                            &mut req,
                            AuthIdentity {
                                user_id,
                                login: user.login,
                                role: user.role,
                                provider: "user-token".to_string(),
                                group_id: None,
                            },
                            AuthMethod::UserToken,
                        );
                        return next.run(req).await;
                    }
                    Ok(Some(_)) => {
                        warn!(user_id = %user_id, path = %path, "Auth middleware: user token names a disabled account");
                    }
                    Ok(None) => {
                        warn!(user_id = %user_id, path = %path, "Auth middleware: user token names an unknown user");
                    }
                    Err(e) => {
                        warn!(error = %e, path = %path, "Auth middleware: user lookup failed for user token");
                    }
                },
                Err(_) => {
                    warn!(path = %path, "Auth middleware: user token user_id is not a uuid");
                }
            }
        }
    }

    // Try opaque auth-session cookies. The database row is authoritative, so
    // revocation and account disablement take effect on the next request.
    let tokens = get_all_cookie_values(req.headers(), &state.config.auth.session_cookie_name);

    for token in &tokens {
        match session::validate_session(
            state.sessions.as_ref(),
            token,
            state.config.auth.session_max_age_secs,
        )
        .await
        {
            Ok(Some(auth_session)) => {
                if is_unsafe(req.method())
                    && !valid_csrf(req.headers(), &auth_session.csrf_token_hash, &state.config)
                {
                    warn!(path = %path, "Auth middleware: CSRF validation failed");
                    return forbidden_response("CSRF validation failed");
                }
                debug!(user_id = %auth_session.user_id, path = %path, "Auth middleware: auth session valid");
                // Stamp user_id onto the surrounding request-trace span so
                // every downstream log line carries it.
                tracing::Span::current()
                    .record("user_id", auth_session.user_id.to_string().as_str());
                insert_auth_extensions(
                    &mut req,
                    AuthIdentity {
                        user_id: auth_session.user_id,
                        login: auth_session.login,
                        role: auth_session.role,
                        provider: auth_session.authenticated_via,
                        group_id: auth_session.group_id,
                    },
                    AuthMethod::SessionCookie,
                );
                return next.run(req).await;
            }
            Ok(None) | Err(_) => continue,
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

fn is_unsafe(method: &Method) -> bool {
    !matches!(method, &Method::GET | &Method::HEAD | &Method::OPTIONS)
}

fn valid_csrf(headers: &axum::http::HeaderMap, expected_hash: &[u8], config: &Config) -> bool {
    let Some(header_token) = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(cookie_token) = get_cookie_value(headers, CSRF_COOKIE_NAME) else {
        return false;
    };
    if header_token != cookie_token || session::hash_token(header_token) != expected_hash {
        return false;
    }

    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| {
            origin.trim_end_matches('/') == config.auth.frontend_url.trim_end_matches('/')
                || config
                    .auth
                    .csrf_trusted_origins
                    .iter()
                    .any(|allowed| allowed.matches(origin))
        })
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

fn forbidden_response(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorBody {
            error: message.to_string(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_authentication_entry_routes_are_exempt() {
        assert!(!is_exempt("/api/v1/auth/me"));
        assert!(!is_exempt("/api/v1/auth/verify"));
        assert!(is_exempt("/api/v1/auth/login"));
        assert!(is_exempt("/api/v1/auth/auth0"));
        assert!(is_exempt("/api/v1/auth/auth0/callback"));
        assert!(is_exempt("/api/v1/auth/oidc/company/start"));
        assert!(is_exempt("/api/v1/auth/oidc/company/callback"));
        assert!(!is_exempt("/api/v1/auth/logout"));
        assert!(!is_exempt("/api/v1/auth/api-keys"));
        assert!(!is_exempt("/api/v1/auth/api-key/session"));
    }

    #[test]
    fn csrf_requires_the_cookie_header_binding_and_same_origin() {
        let config = Config::test_minimal();
        let token = "csrf-token";
        let expected = session::hash_token(token);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{CSRF_COOKIE_NAME}={token}").parse().unwrap(),
        );
        headers.insert("x-csrf-token", token.parse().unwrap());
        headers.insert(header::ORIGIN, config.auth.frontend_url.parse().unwrap());
        assert!(valid_csrf(&headers, &expected, &config));

        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!valid_csrf(&headers, &expected, &config));
        headers.insert("x-csrf-token", "another-token".parse().unwrap());
        assert!(!valid_csrf(&headers, &expected, &config));
    }

    #[test]
    fn csrf_accepts_a_trusted_origin_that_is_not_the_frontend_url() {
        let mut config = Config::test_minimal();
        config.auth.csrf_trusted_origins =
            moatless_types::auth::parse_allowed_origins("https://moatless-t3.example.com");
        let token = "csrf-token";
        let expected = session::hash_token(token);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{CSRF_COOKIE_NAME}={token}").parse().unwrap(),
        );
        headers.insert("x-csrf-token", token.parse().unwrap());

        // The T3 host is not `frontend_url` but is trusted, so it passes.
        headers.insert(
            header::ORIGIN,
            "https://moatless-t3.example.com".parse().unwrap(),
        );
        assert!(valid_csrf(&headers, &expected, &config));

        // An origin on neither list still fails.
        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!valid_csrf(&headers, &expected, &config));
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
