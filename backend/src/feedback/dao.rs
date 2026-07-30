//! Feedback DAO — queries for feedback persistence.

use async_trait::async_trait;

use super::models::Feedback;
use crate::db::DbPool;

/// DAO trait for feedback persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait FeedbackDao: Send + Sync {
    /// Insert a new feedback row.
    async fn insert(&self, row: &Feedback) -> Result<(), sqlx::Error>;
    /// Find a feedback row by its public `feedback_id`.
    async fn find_by_id(&self, feedback_id: &str) -> Result<Option<Feedback>, sqlx::Error>;
    /// List all feedback rows for a task, ordered oldest → newest.
    async fn list_for_task(&self, task_id: &str) -> Result<Vec<Feedback>, sqlx::Error>;
}

/// PostgreSQL implementation of [`FeedbackDao`].
pub struct PgFeedbackDao(pub DbPool);

#[async_trait]
impl FeedbackDao for PgFeedbackDao {
    async fn insert(&self, row: &Feedback) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO feedback (
                feedback_id, task_id, message_uuid, tool_use_id,
                source, surface, comment, metadata, created_by, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(&row.feedback_id)
        .bind(&row.task_id)
        .bind(&row.message_uuid)
        .bind(&row.tool_use_id)
        .bind(&row.source)
        .bind(&row.surface)
        .bind(&row.comment)
        .bind(&row.metadata)
        .bind(&row.created_by)
        .bind(row.created_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn find_by_id(&self, feedback_id: &str) -> Result<Option<Feedback>, sqlx::Error> {
        sqlx::query_as::<_, Feedback>(
            r#"SELECT
                feedback_id, task_id, message_uuid, tool_use_id,
                source, surface, comment, metadata, created_by, created_at
               FROM feedback
               WHERE feedback_id = $1"#,
        )
        .bind(feedback_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn list_for_task(&self, task_id: &str) -> Result<Vec<Feedback>, sqlx::Error> {
        sqlx::query_as::<_, Feedback>(
            r#"SELECT
                feedback_id, task_id, message_uuid, tool_use_id,
                source, surface, comment, metadata, created_by, created_at
               FROM feedback
               WHERE task_id = $1
               ORDER BY created_at ASC, id ASC"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }
}
