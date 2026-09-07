//! API keys table queries.

use async_trait::async_trait;
use chrono::{DateTime, Utc};


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

pub use super::api_keys_dao::PgApiKeyDao;
