//! Request extractors for authentication via Traefik ForwardAuth headers.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// Authenticated user extracted from ForwardAuth headers.
///
/// Traefik's ForwardAuth middleware calls our `/auth/verify` endpoint,
/// which sets `X-User-Id`, `X-User-Login`, etc. headers on the
/// proxied request. This extractor reads those headers.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AuthUser {
    pub user_id: String,
    pub login: String,
    pub role: Option<String>,
    pub provider: Option<String>,
    pub group_id: Option<String>,
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
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user_id = parts
            .headers
            .get("x-user-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| AuthError("Missing X-User-Id header".to_string()))?;

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

        Ok(AuthUser {
            user_id,
            login,
            role,
            provider,
            group_id,
        })
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
