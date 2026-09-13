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

/// Lightweight provider token (`access_token` only) for credential resolution.
#[derive(sqlx::FromRow)]
pub struct ProviderAccessToken {
    pub access_token: String,
}

impl std::fmt::Debug for ProviderAccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderAccessToken")
            .field("access_token", &"[REDACTED]")
            .finish()
    }
}

/// State of the GitHub `provider_tokens` row belonging to one user.
///
/// Carries no token material, so reading it needs no decrypt and no `mark_used`
/// write; `/me` can report setup state on every page load without touching the
/// secrets table or GitHub.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct GithubTokenPresence {
    pub auth_method: Option<String>,
    /// `metadata->>'login'` — the GitHub username, when known.
    pub login: Option<String>,
    /// True when the refresher stamped `metadata.refresh_failure`, meaning the
    /// user must sign in again.
    pub refresh_failure: bool,
}

/// Full provider token row from the `provider_tokens` table.
#[derive(sqlx::FromRow, Clone)]
pub struct ProviderToken {
    pub user_id: String,
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub auth_method: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    /// Provider-side identifier: for bot users (`auth_method =
    /// 'github_app_installation'`) the `GitHubAppRegistry` key of the App
    /// backing the bot; for regular OAuth users, the provider's user id.
    pub provider_user_id: Option<String>,
    pub provider_login: Option<String>,
}

impl std::fmt::Debug for ProviderToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderToken")
            .field("user_id", &self.user_id)
            .field("provider", &self.provider)
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("token_type", &self.token_type)
            .field("auth_method", &self.auth_method)
            .field("expires_at", &self.expires_at)
            .field("metadata", &self.metadata)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("provider_user_id", &self.provider_user_id)
            .field("provider_login", &self.provider_login)
            .finish()
    }
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
    /// Refresh token (GitHub App user tokens support refresh).
    pub refresh_token: Option<String>,
    /// Token TTL in seconds (GitHub App user tokens expire).
    pub expires_in: Option<u64>,
    /// Refresh token TTL in seconds.
    pub refresh_token_expires_in: Option<u64>,
}

/// Normalized user info from an OAuth provider.
pub struct ProviderUserInfo {
    pub id: String,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}
