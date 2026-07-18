//! Request extractors for authentication.
//!
//! Three auth paths (tried in order):
//! 1. **AuthIdentity extension** (production): set by auth middleware after validating
//!    session cookie or API key.
//! 2. **X-User-Token header** (sandbox→backend): JWT-based, for requests that bypass
//!    the auth middleware.
//! 3. **Dev-mode headers / fallback** (no auth provider): reads `X-User-*` headers for
//!    testing, or defaults to `local-user`.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::auth::identity::AuthIdentity;
use crate::auth::models::UserRole;
use crate::sandbox::jwt;
use crate::state::AppState;
use moatless_backend_core::scope::ScopeContext;

/// Authenticated user extracted from request extensions, user token, or dev-mode headers.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub login: String,
    pub role: Option<String>,
    pub provider: Option<String>,
    /// Deprecated: use `team_ids` instead. Kept for backwards compatibility.
    pub group_id: Option<String>,
    /// Team IDs the user belongs to (populated from X-Team-Ids header).
    pub team_ids: Vec<String>,
    pub task_id: Option<String>,
    /// True when this request was authenticated via the sandbox-issued
    /// `X-User-Token` JWT (i.e. came from inside a sandbox pod, on behalf
    /// of the agent's CLI). Used to mark feedback rows as `source = 'agent'`.
    pub is_agent_context: bool,
}

impl AuthUser {
    /// Build a `ScopeContext` from this authenticated user.
    pub fn scope_context(&self) -> ScopeContext {
        ScopeContext {
            user_id: self.user_id.clone(),
            team_ids: self.team_ids.clone(),
            is_admin: self.role.as_deref() == Some("admin"),
        }
    }
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
        // Read X-Task-Id header (used by all auth paths)
        let task_id = parts
            .headers
            .get("x-task-id")
            .and_then(|v| v.to_str().ok())
            .map(std::string::ToString::to_string);

        // Path 1: AuthIdentity from middleware (production — auth provider configured)
        if let Some(identity) = parts.extensions.get::<AuthIdentity>() {
            return Ok(AuthUser {
                user_id: identity.user_id.clone(),
                login: identity.login.clone(),
                role: Some(identity.role.as_str().to_string()),
                provider: Some(identity.provider.clone()),
                group_id: identity.group_id.clone(),
                team_ids: identity.team_ids.clone(),
                task_id,
                is_agent_context: false,
            });
        }

        // Path 2: X-User-Token header (sandbox→backend JWT auth — bypasses middleware)
        if let Some(token) = parts
            .headers
            .get("x-user-token")
            .and_then(|v| v.to_str().ok())
        {
            let secret = state
                .config
                .sandbox
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
                team_ids: vec![],
                task_id,
                is_agent_context: true,
            });
        }

        // Path 3: Dev mode (no auth provider — Docker Compose / local dev)
        // In dev mode, middleware doesn't run, so allow X-User-* headers for testing.
        if state.config.auth.provider.is_none() {
            if let Some(user_id) = parts
                .headers
                .get("x-user-id")
                .and_then(|v| v.to_str().ok())
                .map(std::string::ToString::to_string)
            {
                let login = parts
                    .headers
                    .get("x-user-login")
                    .and_then(|v| v.to_str().ok())
                    .map_or_else(|| user_id.clone(), std::string::ToString::to_string);

                let role = parts
                    .headers
                    .get("x-user-role")
                    .and_then(|v| v.to_str().ok())
                    .map(std::string::ToString::to_string);

                let provider = parts
                    .headers
                    .get("x-user-provider")
                    .and_then(|v| v.to_str().ok())
                    .map(std::string::ToString::to_string);

                let group_id = parts
                    .headers
                    .get("x-group-id")
                    .and_then(|v| v.to_str().ok())
                    .map(std::string::ToString::to_string);

                let team_ids = parts
                    .headers
                    .get("x-team-ids")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| {
                        s.split(',')
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();

                return Ok(AuthUser {
                    user_id,
                    login,
                    role,
                    provider,
                    group_id,
                    team_ids,
                    task_id,
                    is_agent_context: false,
                });
            }

            // No headers provided — default local-user
            return Ok(AuthUser {
                user_id: "local-user".to_string(),
                login: "local-user".to_string(),
                role: Some("admin".to_string()),
                provider: Some("none".to_string()),
                group_id: None,
                team_ids: vec![],
                task_id,
                is_agent_context: false,
            });
        }

        // No valid auth found
        Err(AuthError("Authentication required".to_string()))
    }
}

/// Optional auth extractor for dev mode / unauthenticated endpoints.
///
/// Returns `None` if auth headers are missing instead of 401.
#[derive(Debug, Clone)]
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

/// State-agnostic optional-scope extractor for routers whose state is **not**
/// `AppState` (e.g. the sandbox router's `SandboxApiState`).
///
/// Reads the canonical [`AuthIdentity`] that `auth_middleware` injects after
/// validating a session cookie or API key. Because it only trusts the
/// middleware-set extension (never raw `X-User-*` headers), it is safe on any
/// router state.
///
/// `None` means no identity was attached, which only happens in dev mode
/// (no auth provider → middleware disabled), where every caller is treated as a
/// local admin. When `Some`, callers are scoped to what they may access.
#[derive(Debug, Clone)]
pub struct OptionalScope(pub Option<ScopeContext>);

impl<S: Send + Sync> FromRequestParts<S> for OptionalScope {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<AuthIdentity>()
            .map(|id| ScopeContext {
                user_id: id.user_id.clone(),
                team_ids: id.team_ids.clone(),
                is_admin: id.role == UserRole::Admin,
            });
        Ok(OptionalScope(ctx))
    }
}

/// Extractor that builds a `ScopeContext` from the authenticated user.
impl FromRequestParts<AppState> for ScopeContext {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth = AuthUser::from_request_parts(parts, state).await?;
        Ok(auth.scope_context())
    }
}
