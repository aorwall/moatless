//! Auth API request/response schemas.

use serde::{Deserialize, Deserializer, Serialize};

use crate::enums::UserRole;
use crate::setup::SetupStatus;

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
    /// What the environment still needs before it can do its job. Populated
    /// only by `GET /api/v1/auth/me`; absent on login and profile-update
    /// responses, which the frontend treats as "nothing to report".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupStatus>,
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
/// With `isBot: true`, creates a bot user backed by a configured GitHub App:
/// the server resolves the App's `{slug}[bot]` identity from
/// `GitHubAppRegistry` and writes both the `users` row and the matching
/// `provider_tokens` row atomically. `githubAppKey` is required in this mode.
///
/// With `isBot: false`, creates a human user with a local password. Only
/// available when the server runs local password auth; `login` and `password`
/// are required in this mode.
///
/// `Debug` is hand-written to redact `password`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    /// `true` for a GitHub App bot, `false` for a local password user.
    pub is_bot: bool,
    /// Registry key of a configured GitHub App (e.g. `"dev-bot"`). Required
    /// when `isBot` is `true`.
    #[serde(default)]
    pub github_app_key: Option<String>,
    /// Login handle for a local user. Required when `isBot` is `false`.
    #[serde(default)]
    pub login: Option<String>,
    /// Initial password for a local user. Required when `isBot` is `false`.
    #[serde(default)]
    pub password: Option<String>,
    /// Role to assign. Defaults to `user`.
    #[serde(default)]
    pub role: Option<UserRole>,
    /// Force a password change on first login. Defaults to `false`.
    #[serde(default)]
    pub must_change_password: Option<bool>,
}

impl std::fmt::Debug for CreateUserRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateUserRequest")
            .field("is_bot", &self.is_bot)
            .field("github_app_key", &self.github_app_key)
            .field("login", &self.login)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("role", &self.role)
            .field("must_change_password", &self.must_change_password)
            .finish()
    }
}

/// POST /api/v1/auth/password/login request.
///
/// `Debug` is hand-written to redact `password`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PasswordLoginRequest {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for PasswordLoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordLoginRequest")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// PUT /api/v1/auth/password request (change own password).
///
/// `Debug` is hand-written to redact both passwords.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

impl std::fmt::Debug for ChangePasswordRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangePasswordRequest")
            .field("current_password", &"[REDACTED]")
            .field("new_password", &"[REDACTED]")
            .finish()
    }
}

/// PUT /api/v1/users/{login}/password request (admin reset — no current
/// password required).
///
/// `Debug` is hand-written to redact `new_password`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdminResetPasswordRequest {
    pub new_password: String,
    /// Force a password change on next login. Defaults to `false`.
    #[serde(default)]
    pub must_change_password: Option<bool>,
}

impl std::fmt::Debug for AdminResetPasswordRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminResetPasswordRequest")
            .field("new_password", &"[REDACTED]")
            .field("must_change_password", &self.must_change_password)
            .finish()
    }
}

/// How this deployment authenticates users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    /// Local username/password against `user_credentials`.
    Password,
    /// Redirect to a configured OAuth provider.
    Oauth,
}

/// GET /api/v1/auth/mode response.
///
/// A dedicated endpoint rather than a field on `SessionResponse`: `/auth/me`
/// 401s for anonymous callers, and the login page is anonymous by definition.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AuthModeResponse {
    pub mode: AuthMode,
    /// Provider identifier (`local`, `github`, `github_app`, `auth0`).
    pub provider: String,
    /// Where to send the browser to start an OAuth login. `None` in password mode.
    pub login_url: Option<String>,
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
