//! Persistence helpers for `task_subscriptions`.

use crate::db::DbPool;

use super::models::TaskSubscription;

pub struct PgTaskSubscriptionRepository(pub DbPool);

impl PgTaskSubscriptionRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<TaskSubscription>, sqlx::Error> {
        sqlx::query_as::<_, TaskSubscription>("SELECT * FROM task_subscriptions WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    pub async fn find_by_subject(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Vec<TaskSubscription>, sqlx::Error> {
        sqlx::query_as::<_, TaskSubscription>(
            r#"SELECT * FROM task_subscriptions
               WHERE provider = $1 AND subject_kind = $2 AND subject_key = $3
               ORDER BY created_at ASC"#,
        )
        .bind(provider)
        .bind(subject_kind)
        .bind(subject_key)
        .fetch_all(&self.0)
        .await
    }

    pub async fn find_all_by_subject_excluding_task(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
        exclude_task_id: &str,
    ) -> Result<Vec<TaskSubscription>, sqlx::Error> {
        sqlx::query_as::<_, TaskSubscription>(
            r#"SELECT * FROM task_subscriptions
               WHERE provider = $1
                 AND subject_kind = $2
                 AND subject_key = $3
                 AND task_id <> $4
               ORDER BY created_at ASC"#,
        )
        .bind(provider)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(exclude_task_id)
        .fetch_all(&self.0)
        .await
    }

    pub async fn find_by_task_id(
        &self,
        task_id: &str,
    ) -> Result<Vec<TaskSubscription>, sqlx::Error> {
        sqlx::query_as::<_, TaskSubscription>(
            r#"SELECT * FROM task_subscriptions
               WHERE task_id = $1
               ORDER BY created_at ASC"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }

    pub async fn find_by_task_id_and_subject(
        &self,
        task_id: &str,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskSubscription>, sqlx::Error> {
        sqlx::query_as::<_, TaskSubscription>(
            r#"SELECT * FROM task_subscriptions
               WHERE task_id = $1
                 AND provider = $2
                 AND subject_kind = $3
                 AND subject_key = $4
               LIMIT 1"#,
        )
        .bind(task_id)
        .bind(provider)
        .bind(subject_kind)
        .bind(subject_key)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn find_ingress_created_for_subject(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskSubscription>, sqlx::Error> {
        sqlx::query_as::<_, TaskSubscription>(
            r#"SELECT * FROM task_subscriptions
               WHERE provider = $1
                 AND subject_kind = $2
                 AND subject_key = $3
                 AND created_reason = 'ingress'
               ORDER BY created_at ASC
               LIMIT 1"#,
        )
        .bind(provider)
        .bind(subject_kind)
        .bind(subject_key)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn upsert(&self, row: &TaskSubscription) -> Result<TaskSubscription, sqlx::Error> {
        sqlx::query_as::<_, TaskSubscription>(
            r#"INSERT INTO task_subscriptions
               (id, task_id, provider, subject_kind, subject_key, connection_id,
                ingress_rule_id, reply_target, metadata, last_channel_external_id,
                created_reason, created_at, last_event_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               ON CONFLICT (task_id, provider, subject_kind, subject_key) DO UPDATE
               SET connection_id            = EXCLUDED.connection_id,
                   ingress_rule_id          = COALESCE(task_subscriptions.ingress_rule_id, EXCLUDED.ingress_rule_id),
                   reply_target             = EXCLUDED.reply_target,
                   metadata                 = task_subscriptions.metadata || EXCLUDED.metadata,
                   last_channel_external_id = COALESCE(EXCLUDED.last_channel_external_id, task_subscriptions.last_channel_external_id),
                   last_event_at            = GREATEST(task_subscriptions.last_event_at, EXCLUDED.last_event_at)
               RETURNING *"#,
        )
        .bind(&row.id)
        .bind(&row.task_id)
        .bind(&row.provider)
        .bind(&row.subject_kind)
        .bind(&row.subject_key)
        .bind(&row.connection_id)
        .bind(&row.ingress_rule_id)
        .bind(&row.reply_target)
        .bind(&row.metadata)
        .bind(&row.last_channel_external_id)
        .bind(&row.created_reason)
        .bind(row.created_at)
        .bind(row.last_event_at)
        .fetch_one(&self.0)
        .await
    }

    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM task_subscriptions WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn touch_last_event(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE task_subscriptions SET last_event_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    /// Stamp the latest external id we've seen on this subscription so context
    /// replay can skip the agent's own posts. Also bumps `last_event_at`.
    pub async fn touch_cursor(
        &self,
        id: &str,
        last_channel_external_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE task_subscriptions
               SET last_channel_external_id = $2,
                   last_event_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(last_channel_external_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }
}
