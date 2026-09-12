//! Local email/password login, password change, and the admin reset.

use super::session::{
    authenticate_from_cookie, authenticate_request, issued_session_headers, request_source,
};
use crate::auth::accounts::require_local_auth;
use crate::auth::cookie;
use crate::auth::error::AuthError;
use crate::auth::schemas::{
    AdminResetPasswordRequest, ChangePasswordRequest, MessageResponse, PasswordLoginRequest,
    SessionResponse,
};
use crate::auth::state::AuthState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use moatless_api_schemas::error::ErrorBody;

/// Log in with an email and password, returning an auth-session cookie.
#[utoipa::path(
    post,
    path = "/api/v1/auth/password/login",
    tag = "auth",
    request_body = PasswordLoginRequest,
    responses(
        (status = 200, description = "Logged in", body = SessionResponse),
        (status = 401, description = "Invalid credentials", body = ErrorBody),
        (status = 404, description = "Password auth not enabled", body = ErrorBody),
        (status = 429, description = "Too many failed attempts", body = ErrorBody),
    )
)]
pub(in crate::auth) async fn password_login_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<PasswordLoginRequest>,
) -> Result<Response, AuthError> {
    let present_token = cookie::get_cookie_value(&headers, &state.config.auth.session_cookie_name);
    let (issued, response) = crate::auth::password_operations::login(
        &state,
        req,
        request_source(&headers),
        present_token.as_deref(),
    )
    .await?;
    Ok((issued_session_headers(&state, &issued), Json(response)).into_response())
}

/// Change your own password.
#[utoipa::path(
    put,
    path = "/api/v1/auth/password",
    tag = "auth",
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Password changed", body = MessageResponse),
        (status = 400, description = "Password too short", body = ErrorBody),
        (status = 401, description = "Not authenticated or wrong current password", body = ErrorBody),
        (status = 404, description = "Password auth not enabled", body = ErrorBody),
    )
)]
pub(in crate::auth) async fn change_password_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    require_local_auth(&state)?;
    // Password changes require a session cookie, never an API key.
    let claims = authenticate_from_cookie(&state, &headers).await?;
    crate::auth::password_operations::change_password(&state, claims.sub, req).await?;
    Ok(Json(MessageResponse {
        message: "Password changed successfully".to_string(),
    }))
}

/// Reset another user's password. Admin only — no current password required,
/// so a forgotten password has a recovery path that isn't env surgery.
#[utoipa::path(
    put,
    path = "/api/v1/users/{login}/password",
    tag = "users",
    params(("login" = String, Path, description = "Target user login")),
    request_body = AdminResetPasswordRequest,
    responses(
        (status = 200, description = "Password reset", body = MessageResponse),
        (status = 400, description = "Password too short", body = ErrorBody),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Admin only", body = ErrorBody),
        (status = 404, description = "User has no local credential", body = ErrorBody),
    )
)]
pub(in crate::auth) async fn admin_reset_password_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
    Json(req): Json<AdminResetPasswordRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    require_local_auth(&state)?;
    let identity = authenticate_request(&state, &headers).await?;
    crate::auth::password_operations::reset_password(&state, &identity, &login, req).await?;
    Ok(Json(MessageResponse {
        message: "Password reset successfully".to_string(),
    }))
}
