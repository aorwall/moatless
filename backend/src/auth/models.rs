//! Auth subsystem model structs.
//!
//! Domain row types live in their owning modules (e.g. `task::models::Task`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// =============================================================================
// UserRole
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    User,
    Admin,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
        }
    }
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for UserRole {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "user" => Ok(Self::User),
            "admin" => Ok(Self::Admin),
            other => Err(format!("unknown UserRole: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for UserRole {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for UserRole {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        UserRole::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for UserRole {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

/// Lightweight user row for display name resolution.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct UserDisplay {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
}

// =============================================================================
// Provider tokens — auth subsystem
// =============================================================================

/// Lightweight provider token (access_token only) for credential resolution.
#[derive(sqlx::FromRow, Debug)]
pub struct ProviderAccessToken {
    pub access_token: String,
}

/// Full provider token row from the `provider_tokens` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ProviderToken {
    pub id: i32,
    pub user_id: String,
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// =============================================================================
// OAuth domain types (used by session + provider implementations)
// =============================================================================

/// OAuth tokens from a provider.
pub struct OAuthTokens {
    pub access_token: String,
    pub token_type: String,
    pub scope: Option<String>,
    pub id_token: Option<String>,
}

/// Normalized user info from an OAuth provider.
pub struct ProviderUserInfo {
    pub id: String,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}
