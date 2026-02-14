//! CRUD operations for channel_threads table.

use super::models::ChannelThread;
use crate::db::DbPool;

/// Find a thread by config_id + external_thread_id (unique index).
pub async fn find_by_config_and_thread(
    pool: &DbPool,
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
    .fetch_optional(pool)
    .await
}

/// Find threads by task_id.
pub async fn find_by_task_id(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<ChannelThread>, sqlx::Error> {
    sqlx::query_as::<_, ChannelThread>(
        r#"SELECT id, config_id, task_id, external_thread_id, properties,
                  last_message_at, created_at
           FROM channel_threads WHERE task_id = $1"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// Insert a new channel thread, returning the created row.
pub async fn insert(
    pool: &DbPool,
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
    .fetch_one(pool)
    .await
}

/// Update last_message_at and optionally merge properties.
pub async fn touch(
    pool: &DbPool,
    id: i32,
    properties: Option<&serde_json::Value>,
) -> Result<(), sqlx::Error> {
    if let Some(props) = properties {
        sqlx::query(
            r#"UPDATE channel_threads
               SET last_message_at = now(), properties = properties || $2
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(props)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("UPDATE channel_threads SET last_message_at = now() WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}
