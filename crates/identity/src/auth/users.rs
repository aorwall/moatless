//! User rows, as every consumer of the directory reads them.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use moatless_types::user::{UserRole, UserStatus};

#[derive(sqlx::FromRow, Debug)]
pub struct User {
    pub id: Uuid,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider: String,
    /// The provider's own user id (GitHub numeric id, `local:<login>`,
    /// `bot:<appkey>`, or an Auth0 `sub`). Preserved so login can resolve
    /// `(provider, provider_user_id)` to this internal uuid.
    pub provider_user_id: Option<String>,
    #[sqlx(try_from = "String")]
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub last_login_at: DateTime<Utc>,
    pub settings: serde_json::Value,
    /// `true` when this user row represents a GitHub App bot identity
    /// (`{slug}[bot]`); the App key and token-source live in the user's
    /// `provider_tokens` row (`auth_method = 'github_app_installation'`,
    /// `provider_user_id = <app_key>`).
    pub is_bot: bool,
    #[sqlx(try_from = "String")]
    pub status: UserStatus,
}

/// A provider-specific identity linked to a moatless user.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct UserProviderAccount {
    pub user_id: Uuid,
    pub provider: String,
    pub provider_user_id: String,
    pub created_at: DateTime<Utc>,
}
