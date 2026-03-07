//! Auth API request/response schemas.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::models::UserRole;

// ============================================================================
// Request types
// ============================================================================

/// Query params for OAuth callback.
#[derive(Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    pub state: Option<String>,
}

/// Body for PATCH /api/v1/auth/me.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    pub name: Option<String>,
    pub email: Option<String>,
}

/// Body for POST /api/v1/auth/api-keys.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub scopes: Option<Vec<String>>,
    pub expires_in_days: Option<u32>,
}

// ============================================================================
// Response types
// ============================================================================

/// GET /api/v1/auth/me response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub user: Option<UserInfo>,
    pub expires_at: Option<String>,
}

/// User info within session response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    pub id: String,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider: String,
    pub role: UserRole,
}

/// POST /api/v1/auth/api-keys response (includes the full key, shown once).
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyCreateResponse {
    pub id: String,
    pub key: String,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Option<Vec<String>>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

/// Single item in GET /api/v1/auth/api-keys list.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyListItem {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Option<Vec<String>>,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

/// Single item in GET /api/v1/users list.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserListItem {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

/// GET /api/v1/users response.
#[derive(Serialize, ToSchema)]
pub struct UserListResponse {
    pub users: Vec<UserListItem>,
}

/// POST /api/v1/auth/token/session response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenSessionResponse {
    pub token: String,
    pub user: UserInfo,
    pub expires_at: String,
}

/// Generic message response (logout, revoke, etc.).
#[derive(Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

// ============================================================================
// Helpers for converting DateTime<Utc> to RFC3339 strings
// ============================================================================

/// Format a DateTime<Utc> as an RFC3339 string.
pub fn format_datetime(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

/// Format an optional DateTime<Utc> as an optional RFC3339 string.
pub fn format_datetime_opt(dt: Option<DateTime<Utc>>) -> Option<String> {
    dt.map(format_datetime)
}
