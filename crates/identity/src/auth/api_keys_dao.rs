use async_trait::async_trait;
use moatless_core::db::DbPool;
use super::api_keys::{ApiKey, ApiKeyDao};

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
