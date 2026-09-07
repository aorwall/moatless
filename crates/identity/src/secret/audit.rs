//! Secret audit event helpers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};


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

pub use super::audit_dao::PgSecretAuditDao;
