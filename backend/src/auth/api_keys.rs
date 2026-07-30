//! API keys table queries.

use async_trait::async_trait;
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

/// DAO trait for API key persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ApiKeyDao: Send + Sync {
    async fn insert_api_key(&self, key: &ApiKey) -> Result<(), sqlx::Error>;
    async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, sqlx::Error>;
    async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKey>, sqlx::Error>;
    async fn list_api_keys_by_user(&self, user_id: &str) -> Result<Vec<ApiKey>, sqlx::Error>;
    async fn delete_api_key(&self, key_id: &str) -> Result<(), sqlx::Error>;
    async fn update_last_used(&self, key_id: &str) -> Result<(), sqlx::Error>;
}

/// PostgreSQL implementation of [`ApiKeyDao`].
pub struct PgApiKeyDao(pub DbPool);

#[async_trait]
impl ApiKeyDao for PgApiKeyDao {
    async fn insert_api_key(&self, key: &ApiKey) -> Result<(), sqlx::Error> {
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
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, sqlx::Error> {
        sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE key_hash = $1")
            .bind(key_hash)
            .fetch_optional(&self.0)
            .await
    }

    async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKey>, sqlx::Error> {
        sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE id = $1")
            .bind(key_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn list_api_keys_by_user(&self, user_id: &str) -> Result<Vec<ApiKey>, sqlx::Error> {
        sqlx::query_as::<_, ApiKey>(
            "SELECT * FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.0)
        .await
    }

    async fn delete_api_key(&self, key_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM api_keys WHERE id = $1")
            .bind(key_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn update_last_used(&self, key_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
            .bind(key_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }
}
