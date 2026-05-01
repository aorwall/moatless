//! Message DAO — queries for task message data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::{TaskMessage, TaskMessageContentBlock, TaskMessageContext};
use crate::db::DbPool;

/// Raw tool-call row assembled from the assistant tool_use block plus any
/// matching tool_result row(s). Keeps JSON values un-transformed so the handler
/// can decide which schema to apply.
#[derive(Debug, Clone)]
pub struct ToolCallRow {
    pub tool_use_id: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub result_content: Option<String>,
    pub is_error: Option<bool>,
    pub tool_use_result: Option<serde_json::Value>,
}

/// Repository trait for message persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MessageRepository: Send + Sync {
    async fn get_all(
        &self,
        task_id: &str,
    ) -> Result<
        (
            Vec<TaskMessage>,
            Vec<TaskMessageContext>,
            Vec<TaskMessageContentBlock>,
        ),
        sqlx::Error,
    >;

    async fn append(
        &self,
        msg: &TaskMessage,
        contexts: &[TaskMessageContext],
        content_blocks: &[TaskMessageContentBlock],
    ) -> Result<i32, sqlx::Error>;

    async fn get_pending_user(&self, task_id: &str) -> Result<Vec<TaskMessage>, sqlx::Error>;
    async fn get_last_result(&self, task_id: &str) -> Result<Option<TaskMessage>, sqlx::Error>;
    async fn get_last_user_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error>;
    async fn get_latest_session_id(&self, task_id: &str) -> Result<Option<String>, sqlx::Error>;
    async fn get_latest_agent_session_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, sqlx::Error>;
    async fn set_session_id_for_task(
        &self,
        task_id: &str,
        session_id: &str,
    ) -> Result<u64, sqlx::Error>;
    async fn acknowledge_messages(
        &self,
        task_id: &str,
        uuids: &[String],
        session_id: &str,
    ) -> Result<u64, sqlx::Error>;
    async fn delete_queued(&self, task_id: &str, message_uuid: &str) -> Result<bool, sqlx::Error>;
    async fn get_last_message_type(
        &self,
        task_id: &str,
    ) -> Result<Option<super::models::MessageType>, sqlx::Error>;

    /// Whether the most recent row for a task is a `Result` whose first text
    /// block is "Stopped by user request". Used by `handle_agent_stopped` to
    /// drop duplicate stop fan-out events before they accumulate phantom rows.
    async fn last_is_stopped_marker(&self, task_id: &str) -> Result<bool, sqlx::Error>;
    async fn get_latest_message_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;
    async fn set_channel_external_id(
        &self,
        message_uuid: &str,
        external_id: &str,
    ) -> Result<bool, sqlx::Error>;

    async fn has_channel_external_id(
        &self,
        task_id: &str,
        external_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// Like [`Self::has_channel_external_id`] but matches any of the supplied
    /// candidate ids. Used by the dedup gate to recognize both the new
    /// `<provider>:<kind>:<id>` form and any legacy bare-int form for the
    /// same row in a single round-trip.
    async fn has_any_channel_external_id(
        &self,
        task_id: &str,
        external_ids: &[String],
    ) -> Result<bool, sqlx::Error>;

    async fn get_channel_external_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error>;

    /// Next turn number for a new user-initiated turn: max + 1, or 1 if empty.
    async fn next_turn_number(&self, task_id: &str) -> Result<i32, sqlx::Error>;

    /// Current turn number for in-flight inserts (assistant/system/result rows
    /// that should join the most recent turn): max, or 1 if empty.
    async fn current_turn_number(&self, task_id: &str) -> Result<i32, sqlx::Error>;

    /// Keyset-paginated page of messages strictly older than `before_id`
    /// (or the most recent `limit` rows when `before_id` is `None`).
    /// Returned rows are in chronological (oldest-first) order.
    async fn get_paginated(
        &self,
        task_id: &str,
        before_id: Option<i32>,
        limit: i32,
    ) -> Result<
        (
            Vec<TaskMessage>,
            Vec<TaskMessageContext>,
            Vec<TaskMessageContentBlock>,
        ),
        sqlx::Error,
    >;

    /// Look up a single tool call by its `tool_use_id` within a task. Returns
    /// the assistant's tool_use input plus the matching tool_result content
    /// (if a result has been recorded yet).
    async fn find_tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRow>, sqlx::Error>;
}

/// PostgreSQL implementation of [`MessageRepository`].
pub struct PgMessageRepository(pub DbPool);

#[async_trait]
impl MessageRepository for PgMessageRepository {
    async fn get_all(
        &self,
        task_id: &str,
    ) -> Result<
        (
            Vec<TaskMessage>,
            Vec<TaskMessageContext>,
            Vec<TaskMessageContentBlock>,
        ),
        sqlx::Error,
    > {
        let (messages, contexts, blocks) = tokio::try_join!(
            sqlx::query_as::<_, TaskMessage>(
                r#"SELECT * FROM task_messages
                WHERE task_id = $1
                ORDER BY timestamp, id"#,
            )
            .bind(task_id)
            .fetch_all(&self.0),
            sqlx::query_as::<_, TaskMessageContext>(
                r#"SELECT id, task_message_id, type, reference_id, data, created_at
                FROM task_message_contexts
                WHERE task_id = $1
                ORDER BY id"#,
            )
            .bind(task_id)
            .fetch_all(&self.0),
            sqlx::query_as::<_, TaskMessageContentBlock>(
                r#"SELECT id, task_message_id, sort_order, block_type,
                    show_in_ui, text_content, tool_use_id,
                    tool_result_content, tool_result_is_error,
                    image_media_type, image_data, created_at
                FROM task_message_content_blocks
                WHERE task_id = $1
                ORDER BY task_message_id, sort_order"#,
            )
            .bind(task_id)
            .fetch_all(&self.0),
        )?;

        Ok((messages, contexts, blocks))
    }

    async fn append(
        &self,
        msg: &TaskMessage,
        contexts: &[TaskMessageContext],
        content_blocks: &[TaskMessageContentBlock],
    ) -> Result<i32, sqlx::Error> {
        let mut tx = self.0.begin().await?;

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
                tool_use_result, sdk_metadata, attachments, agent_session_id,
                will_not_resume, run_metrics,
                agent_mode, model, resume, channel_external_id,
                channel_provider, channel_source_name, command_name
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                $21, $22, $23, $24, $25, $26, $27, $28, $29, $30,
                $31, $32, $33, $34, $35, $36, $37
            )
            ON CONFLICT (uuid) DO UPDATE SET
                message_type = EXCLUDED.message_type,
                message_content = EXCLUDED.message_content,
                agent_session_id = COALESCE(EXCLUDED.agent_session_id, task_messages.agent_session_id),
                will_not_resume = COALESCE(EXCLUDED.will_not_resume, task_messages.will_not_resume),
                run_metrics = COALESCE(EXCLUDED.run_metrics, task_messages.run_metrics),
                sdk_metadata = COALESCE(EXCLUDED.sdk_metadata, task_messages.sdk_metadata)
            RETURNING id"#,
        )
        .bind(&msg.task_id)
        .bind(&msg.uuid)
        .bind(msg.message_type)
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
        .bind(msg.message_role)
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
        .bind(&msg.sdk_metadata)
        .bind(&msg.attachments)
        .bind(&msg.agent_session_id)
        .bind(msg.will_not_resume)
        .bind(&msg.run_metrics)
        .bind(&msg.agent_mode)
        .bind(&msg.model)
        .bind(msg.resume)
        .bind(&msg.channel_external_id)
        .bind(&msg.channel_provider)
        .bind(&msg.channel_source_name)
        .bind(&msg.command_name)
        .fetch_one(&mut *tx)
        .await?;

        let message_id = row.0;

        for ctx in contexts {
            sqlx::query(
                r#"INSERT INTO task_message_contexts (task_message_id, task_id, type, reference_id, data)
                   VALUES ($1, $2, $3, $4, $5)"#,
            )
            .bind(message_id)
            .bind(&msg.task_id)
            .bind(&ctx.context_type)
            .bind(&ctx.reference_id)
            .bind(&ctx.data)
            .execute(&mut *tx)
            .await?;
        }

        for block in content_blocks {
            sqlx::query(
                r#"INSERT INTO task_message_content_blocks (
                    task_message_id, task_id, sort_order, block_type, show_in_ui,
                    text_content, tool_use_id, tool_result_content, tool_result_is_error,
                    image_media_type, image_data
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
            )
            .bind(message_id)
            .bind(&msg.task_id)
            .bind(block.sort_order)
            .bind(block.block_type)
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

    async fn get_pending_user(&self, task_id: &str) -> Result<Vec<TaskMessage>, sqlx::Error> {
        sqlx::query_as::<_, TaskMessage>(
            r#"SELECT * FROM task_messages
            WHERE task_id = $1
              AND agent_session_id IS NULL
              AND message_type = 'user'
            ORDER BY timestamp, id"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }

    async fn get_last_result(&self, task_id: &str) -> Result<Option<TaskMessage>, sqlx::Error> {
        sqlx::query_as::<_, TaskMessage>(
            r#"SELECT * FROM task_messages
            WHERE task_id = $1 AND message_type IN ('result', 'error')
            ORDER BY timestamp DESC
            LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn get_last_user_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error> {
        sqlx::query_as::<_, TaskMessage>(
            r#"SELECT * FROM task_messages
            WHERE task_id = $1 AND message_type = 'user'
            ORDER BY timestamp DESC
            LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn get_latest_session_id(&self, task_id: &str) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            r#"SELECT session_id FROM task_messages
               WHERE task_id = $1 AND session_id IS NOT NULL
               ORDER BY timestamp DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;

        Ok(row.and_then(|r| r.0))
    }

    async fn get_latest_agent_session_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            r#"SELECT agent_session_id FROM task_messages
               WHERE task_id = $1 AND agent_session_id IS NOT NULL
               ORDER BY timestamp DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;

        Ok(row.and_then(|r| r.0))
    }

    async fn set_session_id_for_task(
        &self,
        task_id: &str,
        session_id: &str,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE task_messages
               SET session_id = $2, agent_session_id = $2, sent_at = NOW()
               WHERE task_id = $1
                 AND (message_type = 'user' OR message_type = 'error')
                 AND agent_session_id IS NULL"#,
        )
        .bind(task_id)
        .bind(session_id)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected())
    }

    async fn acknowledge_messages(
        &self,
        task_id: &str,
        uuids: &[String],
        session_id: &str,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE task_messages
               SET sent_at = NOW(), session_id = $3, agent_session_id = $3
               WHERE task_id = $1
                 AND uuid = ANY($2)
                 AND agent_session_id IS NULL"#,
        )
        .bind(task_id)
        .bind(uuids)
        .bind(session_id)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected())
    }

    async fn delete_queued(&self, task_id: &str, message_uuid: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"DELETE FROM task_messages
               WHERE task_id = $1 AND uuid = $2 AND agent_session_id IS NULL AND sent_at IS NULL"#,
        )
        .bind(task_id)
        .bind(message_uuid)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_last_message_type(
        &self,
        task_id: &str,
    ) -> Result<Option<super::models::MessageType>, sqlx::Error> {
        let row: Option<(super::models::MessageType,)> = sqlx::query_as(
            r#"SELECT message_type FROM task_messages
               WHERE task_id = $1
               ORDER BY timestamp DESC, id DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(|r| r.0))
    }

    async fn last_is_stopped_marker(&self, task_id: &str) -> Result<bool, sqlx::Error> {
        let row: Option<(super::models::MessageType, serde_json::Value)> = sqlx::query_as(
            r#"SELECT message_type, message_content FROM task_messages
               WHERE task_id = $1
               ORDER BY timestamp DESC, id DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;

        let Some((msg_type, content)) = row else {
            return Ok(false);
        };
        if msg_type != super::models::MessageType::Result {
            return Ok(false);
        }
        let is_stopped = content
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|b| b.get("text"))
            .and_then(|v| v.as_str())
            == Some("Stopped by user request");
        Ok(is_stopped)
    }

    async fn get_latest_message_times(
        &self,
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
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn set_channel_external_id(
        &self,
        message_uuid: &str,
        external_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE task_messages
               SET channel_external_id = $2
               WHERE uuid = $1 AND channel_external_id IS NULL"#,
        )
        .bind(message_uuid)
        .bind(external_id)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn has_channel_external_id(
        &self,
        task_id: &str,
        external_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let row: (bool,) = sqlx::query_as(
            r#"SELECT EXISTS(
                SELECT 1 FROM task_messages
                WHERE task_id = $1 AND channel_external_id = $2
            )"#,
        )
        .bind(task_id)
        .bind(external_id)
        .fetch_one(&self.0)
        .await?;

        Ok(row.0)
    }

    async fn has_any_channel_external_id(
        &self,
        task_id: &str,
        external_ids: &[String],
    ) -> Result<bool, sqlx::Error> {
        if external_ids.is_empty() {
            return Ok(false);
        }
        let row: (bool,) = sqlx::query_as(
            r#"SELECT EXISTS(
                SELECT 1 FROM task_messages
                WHERE task_id = $1 AND channel_external_id = ANY($2)
            )"#,
        )
        .bind(task_id)
        .bind(external_ids)
        .fetch_one(&self.0)
        .await?;

        Ok(row.0)
    }

    async fn get_channel_external_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"SELECT channel_external_id FROM task_messages
               WHERE task_id = $1 AND channel_external_id IS NOT NULL"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn next_turn_number(&self, task_id: &str) -> Result<i32, sqlx::Error> {
        let row: (i32,) = sqlx::query_as(
            r#"SELECT COALESCE(MAX(turn_number), 0) + 1 FROM task_messages WHERE task_id = $1"#,
        )
        .bind(task_id)
        .fetch_one(&self.0)
        .await?;
        Ok(row.0)
    }

    async fn current_turn_number(&self, task_id: &str) -> Result<i32, sqlx::Error> {
        let row: (i32,) = sqlx::query_as(
            r#"SELECT COALESCE(MAX(turn_number), 1) FROM task_messages WHERE task_id = $1"#,
        )
        .bind(task_id)
        .fetch_one(&self.0)
        .await?;
        Ok(row.0)
    }

    async fn get_paginated(
        &self,
        task_id: &str,
        before_id: Option<i32>,
        limit: i32,
    ) -> Result<
        (
            Vec<TaskMessage>,
            Vec<TaskMessageContext>,
            Vec<TaskMessageContentBlock>,
        ),
        sqlx::Error,
    > {
        let mut messages = sqlx::query_as::<_, TaskMessage>(
            r#"SELECT * FROM task_messages
               WHERE task_id = $1
                 AND ($2::int IS NULL OR id < $2)
               ORDER BY id DESC
               LIMIT $3"#,
        )
        .bind(task_id)
        .bind(before_id)
        .bind(limit)
        .fetch_all(&self.0)
        .await?;

        if messages.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }

        // Reverse to chronological (oldest-first) order for the caller.
        messages.reverse();

        let message_ids: Vec<i32> = messages.iter().map(|m| m.id).collect();

        let (contexts, blocks) = tokio::try_join!(
            sqlx::query_as::<_, TaskMessageContext>(
                r#"SELECT id, task_message_id, type, reference_id, data, created_at
                   FROM task_message_contexts
                   WHERE task_id = $1 AND task_message_id = ANY($2)
                   ORDER BY id"#,
            )
            .bind(task_id)
            .bind(&message_ids)
            .fetch_all(&self.0),
            sqlx::query_as::<_, TaskMessageContentBlock>(
                r#"SELECT id, task_message_id, sort_order, block_type,
                       show_in_ui, text_content, tool_use_id,
                       tool_result_content, tool_result_is_error,
                       image_media_type, image_data, created_at
                   FROM task_message_content_blocks
                   WHERE task_id = $1 AND task_message_id = ANY($2)
                   ORDER BY task_message_id, sort_order"#,
            )
            .bind(task_id)
            .bind(&message_ids)
            .fetch_all(&self.0),
        )?;

        Ok((messages, contexts, blocks))
    }

    async fn find_tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRow>, sqlx::Error> {
        // 1. Find the assistant message_content array containing the tool_use block.
        let assistant_row: Option<(serde_json::Value,)> = sqlx::query_as(
            r#"SELECT message_content
               FROM task_messages
               WHERE task_id = $1
                 AND message_content @> jsonb_build_array(
                       jsonb_build_object('id', $2::text, 'type', 'tool_use')
                     )
               ORDER BY id DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .bind(tool_use_id)
        .fetch_optional(&self.0)
        .await?;

        let Some((assistant_content,)) = assistant_row else {
            return Ok(None);
        };

        let (tool_name, tool_input) =
            extract_tool_use_block(&assistant_content, tool_use_id).unwrap_or_else(|| {
                // Defensive fallback: the @> match guarantees the block exists,
                // but the JSON shape may have unexpected keys.
                (String::new(), serde_json::Value::Null)
            });

        // 2. Find any tool_result row referencing the same tool_use_id.
        let result_row: Option<(serde_json::Value, Option<serde_json::Value>)> = sqlx::query_as(
            r#"SELECT message_content, tool_use_result
               FROM task_messages
               WHERE task_id = $1
                 AND message_content @> jsonb_build_array(
                       jsonb_build_object('tool_use_id', $2::text, 'type', 'tool_result')
                     )
               ORDER BY id ASC
               LIMIT 1"#,
        )
        .bind(task_id)
        .bind(tool_use_id)
        .fetch_optional(&self.0)
        .await?;

        let (result_content, is_error, tool_use_result) = match result_row {
            Some((content, raw_typed)) => {
                let (text, err) = extract_tool_result(&content, tool_use_id);
                (text, err, raw_typed)
            }
            None => (None, None, None),
        };

        Ok(Some(ToolCallRow {
            tool_use_id: tool_use_id.to_string(),
            tool_name,
            tool_input,
            result_content,
            is_error,
            tool_use_result,
        }))
    }
}

/// Pull `(name, input)` out of the `tool_use` block matching `tool_use_id`.
fn extract_tool_use_block(
    message_content: &serde_json::Value,
    tool_use_id: &str,
) -> Option<(String, serde_json::Value)> {
    let arr = message_content.as_array()?;
    for block in arr {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
            continue;
        }
        if block.get("id").and_then(|v| v.as_str()) != Some(tool_use_id) {
            continue;
        }
        let name = block
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let input = block.get("input").cloned().unwrap_or(serde_json::Value::Null);
        return Some((name, input));
    }
    None
}

/// Pull `(content, is_error)` out of the `tool_result` block matching `tool_use_id`.
fn extract_tool_result(
    message_content: &serde_json::Value,
    tool_use_id: &str,
) -> (Option<String>, Option<bool>) {
    let Some(arr) = message_content.as_array() else {
        return (None, None);
    };
    for block in arr {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
            continue;
        }
        if block.get("tool_use_id").and_then(|v| v.as_str()) != Some(tool_use_id) {
            continue;
        }
        let content = block
            .get("content")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        let is_error = block.get("is_error").and_then(serde_json::Value::as_bool);
        return (content, is_error);
    }
    (None, None)
}

// =========================================================================
// Free-function wrappers for integration tests
// =========================================================================

/// Append a message with contexts and content blocks (convenience wrapper).
pub async fn append(
    pool: &DbPool,
    msg: &TaskMessage,
    contexts: &[TaskMessageContext],
    content_blocks: &[TaskMessageContentBlock],
) -> Result<i32, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .append(msg, contexts, content_blocks)
        .await
}

/// Get all messages, contexts, and content blocks for a task (convenience wrapper).
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
    PgMessageRepository(pool.clone()).get_all(task_id).await
}

/// Get pending user messages (convenience wrapper).
pub async fn get_pending_user(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<TaskMessage>, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .get_pending_user(task_id)
        .await
}

/// Set session_id for all pending messages (convenience wrapper).
pub async fn set_session_id_for_task(
    pool: &DbPool,
    task_id: &str,
    session_id: &str,
) -> Result<u64, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .set_session_id_for_task(task_id, session_id)
        .await
}

/// Get the last result message (convenience wrapper).
pub async fn get_last_result(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskMessage>, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .get_last_result(task_id)
        .await
}

/// Acknowledge messages with a session_id (convenience wrapper).
pub async fn acknowledge_messages(
    pool: &DbPool,
    task_id: &str,
    uuids: &[String],
    session_id: &str,
) -> Result<u64, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .acknowledge_messages(task_id, uuids, session_id)
        .await
}

/// Delete a queued message (convenience wrapper).
pub async fn delete_queued(
    pool: &DbPool,
    task_id: &str,
    message_uuid: &str,
) -> Result<bool, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .delete_queued(task_id, message_uuid)
        .await
}
