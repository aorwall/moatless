//! CRUD operations for channel_threads table.

use async_trait::async_trait;

use super::models::ChannelThread;
use crate::db::DbPool;

/// Repository trait for channel thread persistence.
///
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&serde_json::Value>` in `touch`.
#[async_trait]
pub trait ChannelThreadRepository: Send + Sync {
    /// Find a thread by config_id + external_thread_id (unique index).
    async fn find_by_config_and_thread(
        &self,
        config_id: &str,
        external_thread_id: &str,
    ) -> Result<Option<ChannelThread>, sqlx::Error>;

    /// Find threads by task_id.
    async fn find_by_task_id(&self, task_id: &str) -> Result<Vec<ChannelThread>, sqlx::Error>;

    /// Find a thread by task_id and config_id.
    async fn find_by_task_id_and_config(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> Result<Option<ChannelThread>, sqlx::Error>;

    /// Insert a new channel thread, returning the created row.
    async fn insert(
        &self,
        config_id: &str,
        task_id: &str,
        external_thread_id: &str,
        properties: &serde_json::Value,
    ) -> Result<ChannelThread, sqlx::Error>;

    /// Update last_message_at and optionally merge properties.
    async fn touch(&self, id: i32, properties: Option<&serde_json::Value>) -> Result<(), sqlx::Error>;

    /// Replace external_thread_id and properties for a pending thread
    /// after the first outbound message has been sent.
    async fn update_thread_info(
        &self,
        id: i32,
        external_thread_id: &str,
        properties: &serde_json::Value,
    ) -> Result<(), sqlx::Error>;
}

/// Flat row returned by the batch channel query (joins threads + configs).
#[derive(sqlx::FromRow, Debug)]
pub struct TaskChannelInfo {
    pub task_id: String,
    pub config_id: String,
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
}

/// Batch-fetch channel info for multiple tasks in a single query.
pub async fn get_channels_for_tasks(
    pool: &crate::db::DbPool,
    task_ids: &[String],
) -> Result<Vec<TaskChannelInfo>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(vec![]);
    }
    sqlx::query_as::<_, TaskChannelInfo>(
        r#"SELECT ct.task_id, cc.id AS config_id, cc.provider, cc.source_id, cc.source_name
           FROM channel_threads ct
           JOIN channel_configs cc ON ct.config_id = cc.id
           WHERE ct.task_id = ANY($1)"#,
    )
    .bind(task_ids)
    .fetch_all(pool)
    .await
}

/// PostgreSQL implementation of [`ChannelThreadRepository`].
pub struct PgChannelThreadRepository(pub DbPool);

#[async_trait]
impl ChannelThreadRepository for PgChannelThreadRepository {
    async fn find_by_config_and_thread(
        &self,
        config_id: &str,
        external_thread_id: &str,
    ) -> Result<Option<ChannelThread>, sqlx::Error> {
        sqlx::query_as::<_, ChannelThread>(
            r#"SELECT id, config_id, task_id, external_thread_id, properties,
                      last_message_at, created_at
               FROM channel_threads
               WHERE config_id = $1 AND external_thread_id = $2"#,
        )
        .bind(config_id)
        .bind(external_thread_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_task_id(&self, task_id: &str) -> Result<Vec<ChannelThread>, sqlx::Error> {
        sqlx::query_as::<_, ChannelThread>(
            r#"SELECT id, config_id, task_id, external_thread_id, properties,
                      last_message_at, created_at
               FROM channel_threads WHERE task_id = $1"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }

    async fn find_by_task_id_and_config(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> Result<Option<ChannelThread>, sqlx::Error> {
        sqlx::query_as::<_, ChannelThread>(
            r#"SELECT id, config_id, task_id, external_thread_id, properties,
                      last_message_at, created_at
               FROM channel_threads WHERE task_id = $1 AND config_id = $2"#,
        )
        .bind(task_id)
        .bind(config_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn insert(
        &self,
        config_id: &str,
        task_id: &str,
        external_thread_id: &str,
        properties: &serde_json::Value,
    ) -> Result<ChannelThread, sqlx::Error> {
        sqlx::query_as::<_, ChannelThread>(
            r#"INSERT INTO channel_threads (config_id, task_id, external_thread_id, properties)
               VALUES ($1, $2, $3, $4)
               RETURNING id, config_id, task_id, external_thread_id, properties,
                         last_message_at, created_at"#,
        )
        .bind(config_id)
        .bind(task_id)
        .bind(external_thread_id)
        .bind(properties)
        .fetch_one(&self.0)
        .await
    }

    async fn touch(&self, id: i32, properties: Option<&serde_json::Value>) -> Result<(), sqlx::Error> {
        if let Some(props) = properties {
            sqlx::query(
                r#"UPDATE channel_threads
                   SET last_message_at = now(), properties = properties || $2
                   WHERE id = $1"#,
            )
            .bind(id)
            .bind(props)
            .execute(&self.0)
            .await?;
        } else {
            sqlx::query("UPDATE channel_threads SET last_message_at = now() WHERE id = $1")
                .bind(id)
                .execute(&self.0)
                .await?;
        }
        Ok(())
    }

    async fn update_thread_info(
        &self,
        id: i32,
        external_thread_id: &str,
        properties: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE channel_threads
               SET external_thread_id = $2, properties = $3, last_message_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(external_thread_id)
        .bind(properties)
        .execute(&self.0)
        .await?;
        Ok(())
    }
}
