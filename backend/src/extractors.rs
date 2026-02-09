//! Request extractors for authentication via Traefik ForwardAuth headers
//! or user auth tokens (for sandbox-to-backend calls).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::sandbox::jwt;
use crate::state::AppState;

/// Authenticated user extracted from ForwardAuth headers or user auth token.
///
/// Two auth paths:
/// 1. **ForwardAuth** (via Traefik): reads `X-User-Id`, `X-User-Login`, etc.
/// 2. **User token** (sandbox→backend): reads `X-User-Token` header, verifies JWT
///
/// The `task_id` field is populated from the `X-Task-Id` header when present.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AuthUser {
    pub user_id: String,
    pub login: String,
    pub role: Option<String>,
    pub provider: Option<String>,
    pub group_id: Option<String>,
    pub task_id: Option<String>,
}

/// Error returned when auth headers are missing.
pub struct AuthError(String);

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, self.0).into_response()
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Read X-Task-Id header (used by both auth paths)
        let task_id = parts
            .headers
            .get("x-task-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Path 1: Try ForwardAuth headers first (X-User-Id present)
        if let Some(user_id) = parts
            .headers
            .get("x-user-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
        {
            let login = parts
                .headers
                .get("x-user-login")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| user_id.clone());

            let role = parts
                .headers
                .get("x-user-role")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let provider = parts
                .headers
                .get("x-user-provider")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let group_id = parts
                .headers
                .get("x-group-id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            return Ok(AuthUser {
                user_id,
                login,
                role,
                provider,
                group_id,
                task_id,
            });
        }

        // Path 2: Try X-User-Token header (sandbox→backend JWT auth)
        if let Some(token) = parts
            .headers
            .get("x-user-token")
            .and_then(|v| v.to_str().ok())
        {
            let secret = state
                .config
                .task_token_secret
                .as_deref()
                .ok_or_else(|| AuthError("Token verification not configured".to_string()))?;

            let claims = jwt::verify_user_auth_token(token, secret)
                .map_err(|e| AuthError(format!("Invalid user token: {e}")))?;

            return Ok(AuthUser {
                user_id: claims.user_id,
                login: claims.login,
                role: None,
                provider: Some("user-token".to_string()),
                group_id: None,
                task_id,
            });
        }

        // Neither auth path succeeded
        Err(AuthError("Missing X-User-Id or X-User-Token header".to_string()))
    }
}

/// Optional auth extractor for dev mode / unauthenticated endpoints.
///
/// Returns `None` if auth headers are missing instead of 401.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MaybeAuthUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for MaybeAuthUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match AuthUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(MaybeAuthUser(Some(user))),
            Err(_) => Ok(MaybeAuthUser(None)),
        }
    }
}
