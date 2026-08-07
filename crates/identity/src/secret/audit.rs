//! Secret audit event helpers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use moatless_core::db::DbPool;

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct SecretAuditRecord {
    pub id: String,
    pub secret_id: Option<String>,
    pub actor_id: String,
    pub action: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub key: String,
    pub result: String,
    pub created_at: DateTime<Utc>,
}

pub struct SecretAuditInput<'a> {
    pub secret_id: Option<&'a str>,
    pub actor_id: &'a str,
    pub action: &'a str,
    pub scope_type: &'a str,
    pub scope_id: Option<&'a str>,
    pub key: &'a str,
    pub result: &'a str,
}

#[async_trait]
pub trait SecretAuditDao: Send + Sync {
    async fn record(&self, input: SecretAuditInput<'_>) -> Result<(), sqlx::Error>;
}

pub struct PgSecretAuditDao(pub DbPool);

#[async_trait]
impl SecretAuditDao for PgSecretAuditDao {
    async fn record(&self, input: SecretAuditInput<'_>) -> Result<(), sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            r#"INSERT INTO secret_audit_events
               (id, secret_id, actor_id, action, scope_type, scope_id, key, result, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())"#,
        )
        .bind(id)
        .bind(input.secret_id)
        .bind(input.actor_id)
        .bind(input.action)
        .bind(input.scope_type)
        .bind(input.scope_id)
        .bind(input.key)
        .bind(input.result)
        .execute(&self.0)
        .await?;
        Ok(())
    }
}
