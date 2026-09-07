use async_trait::async_trait;
use moatless_core::db::DbPool;
use super::audit::{SecretAuditDao, SecretAuditInput};

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
