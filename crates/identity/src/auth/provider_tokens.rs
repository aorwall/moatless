//! Provider tokens table queries.

use async_trait::async_trait;
use chrono::{DateTime, Utc};


/// `provider_tokens.auth_method` value used on bot-user rows. Switches the
/// canonical GitHub-token resolver from "refresh OAuth token" to "mint a fresh
/// installation token from `GitHubAppRegistry`".
pub const AUTH_METHOD_APP_INSTALLATION: &str = "github_app_installation";

/// Documented sentinel written into `provider_tokens.access_token` for
/// installation-backed rows. The real token is minted on demand by the
/// canonical resolver, but any code path that reads this column directly will
/// see this string instead of an empty/blank token — sending it to GitHub
/// yields a clear 401 instead of silently dropping the Authorization header.
pub const APP_INSTALLATION_TOKEN_PLACEHOLDER: &str = "<github-app-installation>";

/// `provider_tokens.auth_method` value for a GitHub App *user* access token —
/// the credential a person gets by authorizing the App, as themselves. The
/// canonical resolver refreshes it near expiry rather than minting a new one.
///
/// Named after the App that issued the token, not after a way of signing in:
/// a row carrying this is unrelated to how its owner authenticated to Moatless.
pub const AUTH_METHOD_APP_USER: &str = "github_app";

/// `provider_tokens.auth_method` value for rows whose real credential lives in
/// the encrypted `secrets` table. The row itself carries only non-secret
/// metadata (`login`, `token_type`) and exists so row-presence consumers —
/// the config endpoint's `connected`, `auth/me`'s `providers[]` — keep working.
pub const AUTH_METHOD_SECRET_REF: &str = "secret_ref";

/// Sentinel written into `provider_tokens.access_token` for `secret_ref` rows.
/// Mirrors [`APP_INSTALLATION_TOKEN_PLACEHOLDER`]: a direct reader of the
/// column sees this instead of a usable token, so sending it to a provider
/// yields a clear 401 rather than silent misbehaviour.
pub const SECRET_REF_TOKEN_PLACEHOLDER: &str = "<secret-ref>";

#[derive(sqlx::FromRow, Debug)]
pub struct ProviderToken {
    pub user_id: String,
    pub provider: String,
    pub auth_method: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub provider_user_id: Option<String>,
    pub provider_login: Option<String>,
    pub connected_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// DAO trait for provider token auth persistence.
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait]
pub trait ProviderTokenAuthDao: Send + Sync {
    async fn upsert_provider_token(&self, token: &ProviderToken) -> Result<(), sqlx::Error>;

    /// Look up the `token_type` for a user+provider row (e.g. "pat" or "bearer").
    /// Used to detect PAT overrides before blindly overwriting during OAuth re-login.
    async fn find_token_type(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<String>, sqlx::Error>;
}

pub use super::provider_tokens_dao::PgProviderTokenAuthDao;
