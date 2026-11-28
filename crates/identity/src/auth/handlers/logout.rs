//! Ending a session, on this device or on all of them.

use super::session::authenticate_from_cookie;
use crate::auth::cookie;
use crate::auth::error::AuthError;
use crate::auth::schemas::MessageResponse;
use crate::auth::sessions as session;
use crate::auth::state::AuthState;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use moatless_types::auth::AuthProvider;
use tracing::info;

async fn do_logout(state: &AuthState, headers: &HeaderMap) -> Result<Response, AuthError> {
    let mut session_provider: Option<AuthProvider> = None;
    if let Some(token) = cookie::get_cookie_value(headers, &state.config.auth.session_cookie_name) {
        if let Some(auth_session) = session::validate_session(
            state.sessions.as_ref(),
            &token,
            state.config.auth.session_max_age_secs,
        )
        .await?
        {
            session_provider = auth_session.authenticated_via.parse().ok();
        }
        session::revoke_session(state.sessions.as_ref(), &token).await?;
    }

    let clear_cookie = cookie::build_clear_session_cookie(&state.config);
    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = clear_cookie.parse() {
        resp_headers.append(header::SET_COOKIE, v);
    }
    if let Ok(v) = cookie::build_clear_csrf_cookie(&state.config).parse() {
        resp_headers.append(header::SET_COOKIE, v);
    }

    // Auth0 requires server-side logout
    if let Some(pid) = session_provider
        && let Some(provider) = state.providers.get(pid)
        && let Some(logout_url) = provider.generate_logout_url(&state.config.auth.frontend_url)
    {
        info!(provider = %pid, "Redirecting to provider logout");
        return Ok((
            StatusCode::FOUND,
            resp_headers,
            [("Location", logout_url.as_str())],
        )
            .into_response());
    }

    Ok((
        resp_headers,
        Json(MessageResponse {
            message: "Logged out successfully".to_string(),
        }),
    )
        .into_response())
}

/// Log out the current user session.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Logged out successfully", body = MessageResponse),
        (status = 302, description = "Redirect to provider logout")
    )
)]
pub(in crate::auth) async fn logout_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    do_logout(&state, &headers).await
}

pub(in crate::auth) async fn logout_get_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    do_logout(&state, &headers).await
}

/// Revoke every auth session belonging to the current user.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout-all",
    tag = "auth",
    responses((status = 200, description = "All sessions revoked", body = MessageResponse))
)]
pub(in crate::auth) async fn logout_all_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let current = authenticate_from_cookie(&state, &headers).await?;
    state
        .sessions
        .delete_sessions_for_user(&current.sub.to_string())
        .await?;
    do_logout(&state, &headers).await
}
