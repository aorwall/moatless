//! Auth subsystem model structs.
//!
//! Domain row types live in their owning modules (e.g. `task::models::Task`).

use chrono::{DateTime, Utc};

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
