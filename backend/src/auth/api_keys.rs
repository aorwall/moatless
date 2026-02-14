//! API keys table queries.

use chrono::{DateTime, Utc};
use crate::db::DbPool;

#[derive(sqlx::FromRow, Debug)]
pub struct ApiKey {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: Option<String>,
    pub group_id: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Insert a new API key.
pub async fn insert_api_key(pool: &DbPool, key: &ApiKey) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO api_keys (id, user_id, name, key_hash, key_prefix, scopes, group_id, last_used_at, expires_at, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(&key.id)
    .bind(&key.user_id)
    .bind(&key.name)
    .bind(&key.key_hash)
    .bind(&key.key_prefix)
    .bind(&key.scopes)
    .bind(&key.group_id)
    .bind(key.last_used_at)
    .bind(key.expires_at)
    .bind(key.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Find an API key by its SHA-256 hash.
pub async fn get_api_key_by_hash(pool: &DbPool, key_hash: &str) -> Result<Option<ApiKey>, sqlx::Error> {
    sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE key_hash = $1")
        .bind(key_hash)
        .fetch_optional(pool)
        .await
}

/// Get API key by ID.
pub async fn get_api_key_by_id(pool: &DbPool, key_id: &str) -> Result<Option<ApiKey>, sqlx::Error> {
    sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE id = $1")
        .bind(key_id)
        .fetch_optional(pool)
        .await
}

/// List all API keys for a user (without revealing the hash).
pub async fn list_api_keys_by_user(pool: &DbPool, user_id: &str) -> Result<Vec<ApiKey>, sqlx::Error> {
    sqlx::query_as::<_, ApiKey>(
        "SELECT * FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Delete an API key by ID.
pub async fn delete_api_key(pool: &DbPool, key_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM api_keys WHERE id = $1")
        .bind(key_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update last_used_at timestamp (fire-and-forget).
pub async fn update_last_used(pool: &DbPool, key_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
        .bind(key_id)
        .execute(pool)
        .await?;
    Ok(())
}
