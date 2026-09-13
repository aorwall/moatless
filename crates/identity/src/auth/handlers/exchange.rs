//! Trading an API key or a pairing code for a session.

use super::session::{issued_session_headers, revoke_present_session};
use crate::auth::api_key;
use crate::auth::error::AuthError;
use crate::auth::schemas::{SessionResponse, TokenSessionResponse, UserInfo, format_datetime};
use crate::auth::sessions as session;
use crate::auth::state::AuthState;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use moatless_api_schemas::error::ErrorBody;

/// Exchange a valid API key for a session cookie (for local dev auth).
#[utoipa::path(
    post,
    path = "/api/v1/auth/api-key/session",
    tag = "auth",
    responses(
        (status = 200, description = "Session created", body = SessionResponse),
        (status = 401, description = "Invalid API key", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn exchange_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthError::Unauthorized)?;

    if !auth_header.starts_with("Bearer mvk_") {
        return Err(AuthError::InvalidApiKey);
    }

    let key = &auth_header[7..]; // strip "Bearer "
    let payload = api_key::validate_key(state.api_keys.clone(), state.users.as_ref(), key)
        .await?
        .ok_or(AuthError::InvalidApiKey)?;

    let user_id = payload
        .user_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AuthError::Unauthorized)?;
    let user = state
        .users
        .get_user(&user_id)
        .await?
        .ok_or(AuthError::Unauthorized)?;

    revoke_present_session(&state, &headers).await?;
    let issued = session::create_session_from_user(
        state.sessions.as_ref(),
        &user,
        payload.group_id.as_deref(),
        state.config.auth.session_max_age_secs,
        state.config.auth.session_absolute_age_secs,
    )
    .await?;

    // Build response from user data + claims
    let body = SessionResponse {
        user: Some(UserInfo {
            id: user.id,
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            provider: "api-key".to_string(),
            role: user.role,
        }),
        expires_at: None,
        providers: Vec::new(),
        setup: None,
    };

    let resp_headers = issued_session_headers(&state, &issued);

    Ok((resp_headers, Json(body)).into_response())
}

/// Exchange a sandbox user auth token (JWT) for a session cookie.
///
/// Sandbox pods carry a `USER_AUTH_TOKEN` JWT signed with `TASK_TOKEN_SECRET`;
/// verifying it lets the CLI call authenticated `/api/v1/*` endpoints.
#[utoipa::path(
    post,
    path = "/api/v1/auth/token/session",
    tag = "auth",
    responses(
        (status = 200, description = "Session created", body = TokenSessionResponse),
        (status = 401, description = "Invalid or missing token", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn exchange_token_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let token = headers
        .get("X-User-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthError::Unauthorized)?;

    let verified = state
        .token_verifier
        .verify_user_token(token)
        .map_err(|e| match e {
            moatless_core::http_auth::TokenVerifyError::NotConfigured => {
                AuthError::Config("TASK_TOKEN_SECRET not configured".to_string())
            }
            moatless_core::http_auth::TokenVerifyError::Invalid(_) => AuthError::Unauthorized,
        })?;

    let user_id = verified
        .user_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AuthError::Unauthorized)?;
    let user = state
        .users
        .get_user(&user_id)
        .await?
        .ok_or(AuthError::Unauthorized)?;

    revoke_present_session(&state, &headers).await?;
    let issued = session::create_session_from_user(
        state.sessions.as_ref(),
        &user,
        None, // sandbox tokens don't carry group_id
        state.config.auth.session_max_age_secs,
        state.config.auth.session_absolute_age_secs,
    )
    .await?;

    let expires_at = chrono::Utc::now()
        + chrono::Duration::seconds(state.config.auth.session_absolute_age_secs as i64);

    let body = TokenSessionResponse {
        token: issued.token.clone(),
        user: UserInfo {
            id: user.id,
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            provider: "token".to_string(),
            role: user.role,
        },
        expires_at: format_datetime(expires_at),
    };

    let resp_headers = issued_session_headers(&state, &issued);

    Ok((resp_headers, Json(body)).into_response())
}
