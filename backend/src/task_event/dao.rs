//! Task event DAO — persistence for lifecycle events.

use async_trait::async_trait;

use super::models::TaskEvent;
use crate::db::DbPool;

/// Insert parameters for a new task event.
pub struct InsertTaskEvent {
    pub event_id: String,
    pub task_id: String,
    pub event_type: String,
    pub sub_type: String,
    pub is_error: bool,
    pub agent_type: Option<String>,
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
    pub timestamp: String,
}

/// Repository trait for task event persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskEventRepository: Send + Sync {
    /// Insert a single event.
    async fn insert_event(&self, event: &InsertTaskEvent) -> Result<(), sqlx::Error>;

    /// List events for a task, ordered by id ascending.
    async fn list_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error>;
}

/// PostgreSQL implementation of [`TaskEventRepository`].
pub struct PgTaskEventRepository(pub DbPool);

#[async_trait]
impl TaskEventRepository for PgTaskEventRepository {
    async fn insert_event(&self, event: &InsertTaskEvent) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO task_events (
                event_id, task_id, event_type, sub_type, is_error,
                agent_type, turn_number, payload, timestamp
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        )
        .bind(&event.event_id)
        .bind(&event.task_id)
        .bind(&event.event_type)
        .bind(&event.sub_type)
        .bind(event.is_error)
        .bind(&event.agent_type)
        .bind(event.turn_number)
        .bind(&event.payload)
        .bind(&event.timestamp)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error> {
        let types_ref: Option<&[String]> = event_types.as_deref();
        sqlx::query_as::<_, TaskEvent>(
            r#"SELECT
                id, event_id, task_id, event_type, sub_type, is_error,
                agent_type, turn_number, payload, timestamp, created_at
            FROM task_events
            WHERE task_id = $1
              AND ($2::text[] IS NULL OR event_type = ANY($2))
            ORDER BY id ASC
            LIMIT $3 OFFSET $4"#,
        )
        .bind(task_id)
        .bind(types_ref)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.0)
        .await
    }
}
