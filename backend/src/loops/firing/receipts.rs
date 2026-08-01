//! Durable adapter-event deduplication.

use crate::db::DbPool;

pub struct PgEventReceiptDao(pub DbPool);

#[async_trait::async_trait]
pub trait EventReceiptDao: Send + Sync {
    async fn record_once(
        &self,
        provider: &str,
        external_event_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<bool, sqlx::Error>;
}

impl PgEventReceiptDao {
    /// Insert an event receipt.
    ///
    /// Returns `true` when this is the first time the event has been seen for
    /// the task, `false` when a unique-key conflict proves it was delivered
    /// already.
    pub async fn record_once(
        &self,
        provider: &str,
        external_event_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"INSERT INTO adapter_event_receipts
               (adapter_kind, external_event_id, subject_kind, subject_key, task_id)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (adapter_kind, external_event_id, task_id) DO NOTHING"#,
        )
        .bind(provider)
        .bind(external_event_id)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(task_id)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected() == 1)
    }
}

#[async_trait::async_trait]
impl EventReceiptDao for PgEventReceiptDao {
    async fn record_once(
        &self,
        provider: &str,
        external_event_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        PgEventReceiptDao::record_once(
            self,
            provider,
            external_event_id,
            subject_kind,
            subject_key,
            task_id,
        )
        .await
    }
}
