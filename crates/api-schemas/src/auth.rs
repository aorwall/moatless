//! Auth API request/response schemas.

use serde::{Deserialize, Deserializer, Serialize};

use crate::enums::UserRole;

// ============================================================================
// Request types
// ============================================================================

/// Query params for OAuth callback.
#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    pub state: Option<String>,
}

/// Query params for CLI login initiation.
#[derive(Debug, Serialize, Deserialize)]
pub struct CliLoginQuery {
    pub port: u16,
}

/// Body for PATCH /api/v1/auth/me.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    pub name: Option<String>,
    pub email: Option<String>,
}

/// Body for PATCH /api/v1/users/{login}.
///
/// `None` means the field was omitted and should remain unchanged.
/// `Some(None)` means the caller explicitly sent `null` and the column should be cleared.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdminUpdateUserRequest {
    #[serde(default, deserialize_with = "deserialize_optional_nullable_field")]
    pub name: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable_field")]
    pub email: Option<Option<String>>,
    pub role: Option<UserRole>,
}

// This endpoint needs a tri-state field: omitted, explicit null, or value.
#[allow(clippy::option_option)]
fn deserialize_optional_nullable_field<'de, D, T>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Body for POST /api/v1/auth/api-keys.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub user: Option<UserInfo>,
    pub expires_at: Option<String>,
    /// Connection state for each external provider (GitHub, etc.). Empty when
    /// no authenticated user. The frontend uses this to surface reconnect
    /// prompts when a provider token has been revoked or failed to refresh.
    #[serde(default)]
    pub providers: Vec<ProviderConnection>,
}

/// Connection state for an external provider (OAuth app, GitHub App, PAT, …).
///
/// `connected = false` indicates the frontend should prompt the user to
/// reconnect: either no token is stored, or the most recent refresh attempt
/// failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnection {
    /// Provider identifier (e.g. `"github"`).
    pub provider: String,
    /// True when a usable token exists and, for refreshable tokens, the most
    /// recent refresh succeeded.
    pub connected: bool,
    /// Provider login (e.g. GitHub username) when available.
    pub login: Option<String>,
}

/// User info within session response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UserListItem {
    pub id: String,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider: String,
    pub role: UserRole,
    pub created_at: String,
    pub last_login_at: String,
    pub is_bot: bool,
}

/// GET /api/v1/users response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UserListResponse {
    pub users: Vec<UserListItem>,
}

/// POST /api/v1/users request.
///
/// Creates a bot user backed by a configured GitHub App. The server resolves
/// the App's `{slug}[bot]` identity from `GitHubAppRegistry` and writes both
/// the `users` row and the matching `provider_tokens` row atomically.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    /// Must be `true`. Human users are created via the OAuth callback, not
    /// this endpoint. (Reserved as a flag so the schema can be extended later
    /// without a breaking change.)
    pub is_bot: bool,
    /// Registry key of a configured GitHub App (e.g. `"dev-bot"`).
    pub github_app_key: String,
}

/// POST /api/v1/users response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateUserResponse {
    pub id: String,
    pub login: String,
    pub email: Option<String>,
    pub is_bot: bool,
    pub provider: String,
    pub role: String,
}

/// POST /api/v1/auth/token/session response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TokenSessionResponse {
    pub token: String,
    pub user: UserInfo,
    pub expires_at: String,
}

/// Generic message response (logout, revoke, etc.).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MessageResponse {
    pub message: String,
}

// ============================================================================
// Helpers for converting DateTime<Utc> to RFC3339 strings
// ============================================================================

/// Format a `DateTime<Utc>` as an RFC3339 string.
pub fn format_datetime(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.to_rfc3339()
}

/// Format an optional `DateTime<Utc>` as an optional RFC3339 string.
pub fn format_datetime_opt(dt: Option<chrono::DateTime<chrono::Utc>>) -> Option<String> {
    dt.map(format_datetime)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::AdminUpdateUserRequest;

    #[test]
    fn admin_update_user_distinguishes_omitted_null_and_value() {
        let omitted: AdminUpdateUserRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(omitted.name, None);
        assert_eq!(omitted.email, None);

        let cleared: AdminUpdateUserRequest =
            serde_json::from_value(json!({ "name": null, "email": null })).unwrap();
        assert_eq!(cleared.name, Some(None));
        assert_eq!(cleared.email, Some(None));

        let updated: AdminUpdateUserRequest =
            serde_json::from_value(json!({ "name": "Ada", "email": "ada@example.com" })).unwrap();
        assert_eq!(updated.name, Some(Some("Ada".to_string())));
        assert_eq!(updated.email, Some(Some("ada@example.com".to_string())));
    }
}
