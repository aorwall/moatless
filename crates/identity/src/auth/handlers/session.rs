//! Who the caller is: the cookies a new session sets, and reading a request
//! back into an `AuthIdentity`.

use super::preview::enforce_preview_access;
use crate::auth::api_key;
use crate::auth::cookie;
use crate::auth::error::AuthError;
use crate::auth::sessions as session;
use crate::auth::sessions::IssuedAuthSession;
use crate::auth::state::AuthState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use moatless_api_schemas::error::ErrorBody;
use moatless_types::access_control::ScopeContext;
use moatless_types::auth::AuthIdentity;
use moatless_types::user::UserRole;

pub(super) fn issued_session_headers(state: &AuthState, issued: &IssuedAuthSession) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let session_cookie = cookie::build_session_cookie(
        &state.config,
        &issued.token,
        state.config.auth.session_absolute_age_secs,
    );
    let csrf_cookie = cookie::build_csrf_cookie(
        &state.config,
        &issued.csrf_token,
        state.config.auth.session_absolute_age_secs,
    );
    if let Ok(value) = session_cookie.parse() {
        headers.append(header::SET_COOKIE, value);
    }
    if let Ok(value) = csrf_cookie.parse() {
        headers.append(header::SET_COOKIE, value);
    }
    headers
}

pub(super) fn request_source(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(super) async fn revoke_present_session(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<(), AuthError> {
    if let Some(token) = cookie::get_cookie_value(headers, &state.config.auth.session_cookie_name) {
        session::revoke_session(state.sessions.as_ref(), &token).await?;
    }
    Ok(())
}

pub(super) struct CookieSession {
    pub(super) sub: uuid::Uuid,
    pub(super) role: UserRole,
    pub(super) provider: String,
    pub(super) group_id: Option<String>,
    pub(super) expires_at: chrono::DateTime<chrono::Utc>,
}

impl CookieSession {
    pub(super) fn user_role(&self) -> UserRole {
        self.role
    }
}

pub(super) async fn authenticate_from_cookie(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<CookieSession, AuthError> {
    let token = cookie::get_cookie_value(headers, &state.config.auth.session_cookie_name)
        .ok_or(AuthError::Unauthorized)?;
    let resolved = session::validate_session(
        state.sessions.as_ref(),
        &token,
        state.config.auth.session_max_age_secs,
    )
    .await?
    .ok_or(AuthError::SessionExpired)?;
    Ok(CookieSession {
        sub: resolved.user_id,
        role: resolved.role,
        provider: resolved.authenticated_via,
        group_id: resolved.group_id,
        expires_at: resolved.absolute_expires_at,
    })
}

/// Authenticate from session cookie or API key.
///
/// Used by both `verify_handler` (`ForwardAuth`) and internal callers.
pub(super) async fn authenticate_request(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, AuthError> {
    // Try API key (Authorization: Bearer mvk_xxx)
    if let Some(auth_header) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if auth_header.starts_with("Bearer mvk_") {
            let key = &auth_header[7..]; // strip "Bearer "
            if let Some(payload) = api_key::validate_key_cached(
                state.api_keys.clone(),
                state.users.as_ref(),
                key,
                &state.cache,
            )
            .await?
            {
                let user_id = payload
                    .user_id
                    .parse::<uuid::Uuid>()
                    .map_err(|_| AuthError::InvalidApiKey)?;
                return Ok(AuthIdentity {
                    user_id,
                    login: payload.user_login,
                    role: payload.role,
                    provider: payload.provider,
                    group_id: payload.group_id,
                });
            }
            return Err(AuthError::InvalidApiKey);
        }
    }

    // Try opaque auth-session cookies.
    let tokens = cookie::get_all_cookie_values(headers, &state.config.auth.session_cookie_name);
    if tokens.is_empty() {
        return Err(AuthError::Unauthorized);
    }

    for token in &tokens {
        if let Some(auth_session) = session::validate_session(
            state.sessions.as_ref(),
            token,
            state.config.auth.session_max_age_secs,
        )
        .await?
        {
            return Ok(AuthIdentity {
                user_id: auth_session.user_id,
                login: auth_session.login,
                role: auth_session.role,
                provider: auth_session.authenticated_via,
                group_id: auth_session.group_id,
            });
        }
    }

    Err(AuthError::SessionExpired)
}

/// Verify authentication via session cookie or API key (Traefik `ForwardAuth`).
///
/// Returns 200/401 only, no response headers; for preview URLs (hostname
/// contains `--`), also checks task-based access.
#[utoipa::path(
    get,
    path = "/api/v1/auth/verify",
    tag = "auth",
    responses(
        (status = 200, description = "Authentication valid"),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Access denied", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn verify_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;

    // For preview URLs, check task-based access control.
    let scope_ctx = ScopeContext {
        user_id: identity.user_id.to_string(),
        is_admin: identity.role == UserRole::Admin,
    };
    let forwarded_host = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok());
    enforce_preview_access(state.task_access.as_ref(), &scope_ctx, forwarded_host).await?;

    Ok((StatusCode::OK, "OK").into_response())
}
