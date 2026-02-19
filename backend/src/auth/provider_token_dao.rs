//! Provider token DAO — queries for credential resolution.

use async_trait::async_trait;

use super::models::{ProviderAccessToken, ProviderToken};
use crate::db::DbPool;

/// Repository trait for provider token credential resolution.
///
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&str>` / `Option<&Value>` in async-trait
/// methods.  Tests use `PgProviderTokenRepository` via `#[sqlx::test]`.
#[async_trait]
pub trait ProviderTokenRepository: Send + Sync {
    /// Find a provider token (access_token only) by user_id and provider name.
    async fn find_provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error>;

    /// Find the full provider token row by user_id and provider name.
    async fn find_provider_token_full(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert a provider token (insert or update on conflict).
    async fn upsert_token(
        &self,
        user_id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Delete a provider token for a user.
    async fn delete_token(&self, user_id: &str, provider: &str) -> Result<(), sqlx::Error>;
}

/// PostgreSQL implementation of [`ProviderTokenRepository`].
pub struct PgProviderTokenRepository(pub DbPool);

#[async_trait]
impl ProviderTokenRepository for PgProviderTokenRepository {
    async fn find_provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderAccessToken>(
            "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_provider_token_full(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderToken>(
            r#"SELECT user_id, provider, access_token, refresh_token, token_type,
                      metadata, created_at, updated_at
               FROM provider_tokens
               WHERE user_id = $1 AND provider = $2"#,
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert_token(
        &self,
        user_id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO provider_tokens (user_id, provider, access_token, refresh_token, token_type, metadata, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, NOW())
               ON CONFLICT (user_id, provider)
               DO UPDATE SET access_token = $3, refresh_token = $4, token_type = $5, metadata = $6, updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(access_token)
        .bind(refresh_token)
        .bind(token_type)
        .bind(metadata)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete_token(&self, user_id: &str, provider: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM provider_tokens WHERE user_id = $1 AND provider = $2")
            .bind(user_id)
            .bind(provider)
            .execute(&self.0)
            .await?;
        Ok(())
    }
}
