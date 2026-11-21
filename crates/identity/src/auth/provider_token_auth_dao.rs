//! The auth-state view over `provider_tokens`: the two writes a login or a
//! connect makes to the row.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;

#[derive(sqlx::FromRow, Debug)]
pub(crate) struct ProviderToken {
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
pub(crate) trait ProviderTokenAuthDao: Send + Sync {
    async fn upsert_provider_token(&self, token: &ProviderToken) -> Result<(), sqlx::Error>;

    /// Look up the `token_type` for a user+provider row (e.g. "pat" or
    /// "bearer"). Used to detect PAT overrides before blindly overwriting
    /// during OAuth re-login.
    async fn find_token_type(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<String>, sqlx::Error>;
}

/// PostgreSQL implementation of [`ProviderTokenAuthDao`].
pub(crate) struct PgProviderTokenAuthDao(pub(crate) DbPool);

#[async_trait]
impl ProviderTokenAuthDao for PgProviderTokenAuthDao {
    async fn upsert_provider_token(&self, token: &ProviderToken) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO provider_tokens (
                user_id, provider, auth_method, access_token, refresh_token,
                token_type, oauth_scope, provider_user_id, provider_login,
                connected_at, expires_at, scope, owner_user_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'user', $1)
            ON CONFLICT (user_id, provider) DO UPDATE SET
                auth_method = EXCLUDED.auth_method,
                access_token = EXCLUDED.access_token,
                refresh_token = EXCLUDED.refresh_token,
                token_type = EXCLUDED.token_type,
                oauth_scope = EXCLUDED.oauth_scope,
                scope = EXCLUDED.scope,
                owner_user_id = EXCLUDED.owner_user_id,
                provider_user_id = COALESCE(EXCLUDED.provider_user_id, provider_tokens.provider_user_id),
                provider_login = COALESCE(EXCLUDED.provider_login, provider_tokens.provider_login),
                connected_at = EXCLUDED.connected_at,
                expires_at = EXCLUDED.expires_at
            "#,
        )
        .bind(&token.user_id)
        .bind(&token.provider)
        .bind(&token.auth_method)
        .bind(&token.access_token)
        .bind(&token.refresh_token)
        .bind(&token.token_type)
        .bind(&token.scope)
        .bind(&token.provider_user_id)
        .bind(&token.provider_login)
        .bind(token.connected_at)
        .bind(token.expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn find_token_type(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT token_type FROM provider_tokens WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.and_then(|r| r.0))
    }
}
