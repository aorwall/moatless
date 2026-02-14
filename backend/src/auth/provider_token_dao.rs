//! Provider token DAO — queries for credential resolution.

use super::models::{ProviderAccessToken, ProviderToken};
use crate::db::DbPool;

/// Find a provider token by user_id and provider name.
///
/// Queries the `provider_tokens` table which stores user-linked OAuth/API tokens
/// for providers like `github` and `claude`.
pub async fn find_provider_token(
    pool: &DbPool,
    user_id: &str,
    provider: &str,
) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
    sqlx::query_as::<_, ProviderAccessToken>(
        "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = $2",
    )
    .bind(user_id)
    .bind(provider)
    .fetch_optional(pool)
    .await
}

/// Find the full provider token row by user_id and provider name.
pub async fn find_provider_token_full(
    pool: &DbPool,
    user_id: &str,
    provider: &str,
) -> Result<Option<ProviderToken>, sqlx::Error> {
    sqlx::query_as::<_, ProviderToken>(
        r#"SELECT id, user_id, provider, access_token, refresh_token, token_type,
                  metadata, created_at, updated_at
           FROM provider_tokens
           WHERE user_id = $1 AND provider = $2"#,
    )
    .bind(user_id)
    .bind(provider)
    .fetch_optional(pool)
    .await
}

/// Upsert a provider token (insert or update on conflict).
pub async fn upsert_token(
    pool: &DbPool,
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
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a provider token for a user.
pub async fn delete_token(
    pool: &DbPool,
    user_id: &str,
    provider: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM provider_tokens WHERE user_id = $1 AND provider = $2")
        .bind(user_id)
        .bind(provider)
        .execute(pool)
        .await?;
    Ok(())
}
