//! Message DAO — queries for task message data access.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::models::{TaskMessageContentBlock, TaskMessageContext, TaskMessage};
use crate::db::DbPool;

/// Get all messages for a task, ordered by timestamp + id.
/// Returns messages, contexts, and content blocks separately for in-memory joining.
pub async fn get_all(
    pool: &DbPool,
    task_id: &str,
) -> Result<
    (
        Vec<TaskMessage>,
        Vec<TaskMessageContext>,
        Vec<TaskMessageContentBlock>,
    ),
    sqlx::Error,
> {
    // Run 3 queries in parallel
    let (messages, contexts, blocks) = tokio::try_join!(
        sqlx::query_as::<_, TaskMessage>(
            r#"SELECT
                id, task_id, uuid, message_type, user_type, parent_uuid,
                is_sidechain, cwd, session_id, turn_number, version,
                git_branch, request_id, timestamp, message_role, message_content,
                assistant_message_id, assistant_message_type, assistant_message_model,
                assistant_stop_reason, assistant_stop_sequence,
                assistant_usage_input_tokens, assistant_usage_output_tokens,
                assistant_usage_cache_creation_input_tokens,
                assistant_usage_cache_read_input_tokens,
                tool_use_result, attachments, agent_session_id,
                turn_status, turn_result, will_not_resume, run_metrics,
                agent_mode, model, resume_session_id, sent_at, created_at
            FROM task_messages
            WHERE task_id = $1
            ORDER BY timestamp, id"#,
        )
        .bind(task_id)
        .fetch_all(pool),
        sqlx::query_as::<_, TaskMessageContext>(
            r#"SELECT c.id, c.task_message_id, c.type, c.reference_id, c.data, c.created_at
            FROM task_message_contexts c
            JOIN task_messages m ON c.task_message_id = m.id
            WHERE m.task_id = $1
            ORDER BY c.id"#,
        )
        .bind(task_id)
        .fetch_all(pool),
        sqlx::query_as::<_, TaskMessageContentBlock>(
            r#"SELECT b.id, b.task_message_id, b.sort_order, b.block_type,
                b.show_in_ui, b.text_content, b.tool_use_id,
                b.tool_result_content, b.tool_result_is_error,
                b.image_media_type, b.image_data, b.created_at
            FROM task_message_content_blocks b
            JOIN task_messages m ON b.task_message_id = m.id
            WHERE m.task_id = $1
            ORDER BY b.task_message_id, b.sort_order"#,
        )
        .bind(task_id)
        .fetch_all(pool),
    )?;

    Ok((messages, contexts, blocks))
}

/// Append a message with its contexts and content blocks in a transaction.
pub async fn append(
    pool: &DbPool,
    msg: &TaskMessage,
    contexts: &[TaskMessageContext],
    content_blocks: &[TaskMessageContentBlock],
) -> Result<i32, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Upsert message (conflict on uuid)
    let row: (i32,) = sqlx::query_as(
        r#"INSERT INTO task_messages (
            task_id, uuid, message_type, user_type, parent_uuid,
            is_sidechain, cwd, session_id, turn_number, version,
            git_branch, request_id, timestamp, message_role, message_content,
            assistant_message_id, assistant_message_type, assistant_message_model,
            assistant_stop_reason, assistant_stop_sequence,
            assistant_usage_input_tokens, assistant_usage_output_tokens,
            assistant_usage_cache_creation_input_tokens,
            assistant_usage_cache_read_input_tokens,
            tool_use_result, attachments, agent_session_id,
            turn_status, turn_result, will_not_resume, run_metrics,
            agent_mode, model, resume_session_id
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
            $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
            $21, $22, $23, $24, $25, $26, $27, $28, $29, $30,
            $31, $32, $33, $34
        )
        ON CONFLICT (uuid) DO UPDATE SET
            message_content = EXCLUDED.message_content,
            agent_session_id = COALESCE(EXCLUDED.agent_session_id, task_messages.agent_session_id),
            turn_status = COALESCE(EXCLUDED.turn_status, task_messages.turn_status),
            turn_result = COALESCE(EXCLUDED.turn_result, task_messages.turn_result),
            will_not_resume = COALESCE(EXCLUDED.will_not_resume, task_messages.will_not_resume),
            run_metrics = COALESCE(EXCLUDED.run_metrics, task_messages.run_metrics)
        RETURNING id"#,
    )
    .bind(&msg.task_id)
    .bind(&msg.uuid)
    .bind(&msg.message_type)
    .bind(&msg.user_type)
    .bind(&msg.parent_uuid)
    .bind(msg.is_sidechain)
    .bind(&msg.cwd)
    .bind(&msg.session_id)
    .bind(msg.turn_number)
    .bind(&msg.version)
    .bind(&msg.git_branch)
    .bind(&msg.request_id)
    .bind(&msg.timestamp)
    .bind(&msg.message_role)
    .bind(&msg.message_content)
    .bind(&msg.assistant_message_id)
    .bind(&msg.assistant_message_type)
    .bind(&msg.assistant_message_model)
    .bind(&msg.assistant_stop_reason)
    .bind(&msg.assistant_stop_sequence)
    .bind(msg.assistant_usage_input_tokens)
    .bind(msg.assistant_usage_output_tokens)
    .bind(msg.assistant_usage_cache_creation_input_tokens)
    .bind(msg.assistant_usage_cache_read_input_tokens)
    .bind(&msg.tool_use_result)
    .bind(&msg.attachments)
    .bind(&msg.agent_session_id)
    .bind(&msg.turn_status)
    .bind(&msg.turn_result)
    .bind(msg.will_not_resume)
    .bind(&msg.run_metrics)
    .bind(&msg.agent_mode)
    .bind(&msg.model)
    .bind(&msg.resume_session_id)
    .fetch_one(&mut *tx)
    .await?;

    let message_id = row.0;

    // Insert contexts
    for ctx in contexts {
        sqlx::query(
            r#"INSERT INTO task_message_contexts (task_message_id, type, reference_id, data)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(message_id)
        .bind(&ctx.context_type)
        .bind(&ctx.reference_id)
        .bind(&ctx.data)
        .execute(&mut *tx)
        .await?;
    }

    // Insert content blocks
    for block in content_blocks {
        sqlx::query(
            r#"INSERT INTO task_message_content_blocks (
                task_message_id, sort_order, block_type, show_in_ui,
                text_content, tool_use_id, tool_result_content, tool_result_is_error,
                image_media_type, image_data
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(message_id)
        .bind(block.sort_order)
        .bind(&block.block_type)
        .bind(block.show_in_ui)
        .bind(&block.text_content)
        .bind(&block.tool_use_id)
        .bind(&block.tool_result_content)
        .bind(block.tool_result_is_error)
        .bind(&block.image_media_type)
        .bind(&block.image_data)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(message_id)
}

/// Get pending user messages (not yet confirmed by agent session).
pub async fn get_pending_user(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<TaskMessage>, sqlx::Error> {
    sqlx::query_as::<_, TaskMessage>(
        r#"SELECT
            id, task_id, uuid, message_type, user_type, parent_uuid,
            is_sidechain, cwd, session_id, turn_number, version,
            git_branch, request_id, timestamp, message_role, message_content,
            assistant_message_id, assistant_message_type, assistant_message_model,
            assistant_stop_reason, assistant_stop_sequence,
            assistant_usage_input_tokens, assistant_usage_output_tokens,
            assistant_usage_cache_creation_input_tokens,
            assistant_usage_cache_read_input_tokens,
            tool_use_result, attachments, agent_session_id,
            turn_status, turn_result, will_not_resume, run_metrics,
            agent_mode, model, resume_session_id, sent_at, created_at
        FROM task_messages
        WHERE task_id = $1 AND agent_session_id IS NULL AND message_type = 'user'
        ORDER BY timestamp, id"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// Get the last result message for a task.
pub async fn get_last_result(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskMessage>, sqlx::Error> {
    sqlx::query_as::<_, TaskMessage>(
        r#"SELECT
            id, task_id, uuid, message_type, user_type, parent_uuid,
            is_sidechain, cwd, session_id, turn_number, version,
            git_branch, request_id, timestamp, message_role, message_content,
            assistant_message_id, assistant_message_type, assistant_message_model,
            assistant_stop_reason, assistant_stop_sequence,
            assistant_usage_input_tokens, assistant_usage_output_tokens,
            assistant_usage_cache_creation_input_tokens,
            assistant_usage_cache_read_input_tokens,
            tool_use_result, attachments, agent_session_id,
            turn_status, turn_result, will_not_resume, run_metrics,
            agent_mode, model, resume_session_id, sent_at, created_at
        FROM task_messages
        WHERE task_id = $1 AND message_type = 'result'
        ORDER BY timestamp DESC
        LIMIT 1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
}

/// Get the last user message for a task.
pub async fn get_last_user_message(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskMessage>, sqlx::Error> {
    sqlx::query_as::<_, TaskMessage>(
        r#"SELECT
            id, task_id, uuid, message_type, user_type, parent_uuid,
            is_sidechain, cwd, session_id, turn_number, version,
            git_branch, request_id, timestamp, message_role, message_content,
            assistant_message_id, assistant_message_type, assistant_message_model,
            assistant_stop_reason, assistant_stop_sequence,
            assistant_usage_input_tokens, assistant_usage_output_tokens,
            assistant_usage_cache_creation_input_tokens,
            assistant_usage_cache_read_input_tokens,
            tool_use_result, attachments, agent_session_id,
            turn_status, turn_result, will_not_resume, run_metrics,
            agent_mode, model, resume_session_id, sent_at, created_at
        FROM task_messages
        WHERE task_id = $1 AND message_type = 'user'
        ORDER BY timestamp DESC
        LIMIT 1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
}

/// Get latest agent session ID.
pub async fn get_latest_session_id(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        r#"SELECT session_id FROM task_messages
           WHERE task_id = $1 AND session_id IS NOT NULL
           ORDER BY timestamp DESC
           LIMIT 1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.and_then(|r| r.0))
}

/// Set session_id, agent_session_id, and sent_at on pending user messages
/// when the agent session starts (session_init event).
pub async fn set_session_id_for_task(
    pool: &DbPool,
    task_id: &str,
    session_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"UPDATE task_messages
           SET session_id = $2, agent_session_id = $2, sent_at = NOW()
           WHERE task_id = $1
             AND message_type = 'user'
             AND agent_session_id IS NULL"#,
    )
    .bind(task_id)
    .bind(session_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Delete a queued message (only if not yet acknowledged).
pub async fn delete_queued(
    pool: &DbPool,
    task_id: &str,
    message_uuid: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"DELETE FROM task_messages
           WHERE task_id = $1 AND uuid = $2 AND agent_session_id IS NULL"#,
    )
    .bind(task_id)
    .bind(message_uuid)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Get the `message_type` of the most recent message for a task.
///
/// Returns `None` if no messages exist.
pub async fn get_last_message_type(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<super::models::MessageType>, sqlx::Error> {
    let row: Option<(super::models::MessageType,)> = sqlx::query_as(
        r#"SELECT message_type FROM task_messages
           WHERE task_id = $1
           ORDER BY timestamp DESC, id DESC
           LIMIT 1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

/// Get the latest message timestamp for each of the given task IDs (batch).
///
/// Task IDs with no messages are absent from the returned map.
pub async fn get_latest_message_times(
    pool: &DbPool,
    task_ids: &[String],
) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
        r#"SELECT task_id, MAX(created_at) AS latest_at
           FROM task_messages
           WHERE task_id = ANY($1)
           GROUP BY task_id"#,
    )
    .bind(task_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().collect())
}
