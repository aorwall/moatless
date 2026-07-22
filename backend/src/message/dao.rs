//! Message DAO — queries for task message data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::{TaskMessage, TaskMessageContentBlock, TaskMessageContext};
use crate::db::DbPool;

/// A page of messages with their associated context rows and content blocks.
type MessagePage = (
    Vec<TaskMessage>,
    Vec<TaskMessageContext>,
    Vec<TaskMessageContentBlock>,
);

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
    /// UUID of the assistant row containing the tool_use block. Surfaced so
    /// post-message hooks can tag the parent message rather than the hidden
    /// System tool_result row.
    pub parent_message_uuid: String,
}

/// Per-connection inbound/outgoing message tally for a task, grouped by the
/// external adapter + subject key the message was routed through. Drives the
/// per-connection `↓in / ↑out` counts on the task-connections admin view.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConnectionMessageCount {
    pub task_id: String,
    pub external_adapter: Option<String>,
    pub external_subject_key: Option<String>,
    /// Inbound rows (`message_role = 'user'`).
    pub incoming: i64,
    /// Outbound rows (`message_role = 'assistant'`).
    pub outgoing: i64,
}

/// Per-`Agent`-call subagent aggregate: message count + summed assistant
/// tokens, keyed by the spawning `Agent` tool-use id (`parent_uuid`). Feeds the
/// `subagentSummary` injected onto the top-level read path and live emit.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SubagentAggregate {
    /// The spawning `Agent` tool-use id (`task_messages.parent_uuid`).
    pub parent_uuid: String,
    /// `COUNT(*)` of the subagent's rows.
    pub message_count: i64,
    /// `SUM(input_tokens + output_tokens)` across the subagent's assistant rows.
    pub tokens: i64,
}

/// One Task carrying unacknowledged user messages, the desired-state
/// reconciler's message-wake input.
///
/// A pending user message is `agent_session_id IS NULL AND message_type = 'user'`
/// — the `task_messages_pending_user_idx` predicate. `newest_created_at` is the
/// DB-owned timestamp of the most recent pending row; the reconciler compares it
/// against `sandbox_desired_state_updated_at` to decide whether a wake supersedes
/// a later stop/remove.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingUserMessages {
    pub task_id: String,
    pub newest_created_at: DateTime<Utc>,
    pub pending_count: i64,
}

/// A user-turn that was acknowledged by the sandbox but never reached a
/// terminal `agent.completed/error/stopped` event — i.e. the sandbox
/// crashed/restarted mid-run. Used by the recovery RPC to drive a
/// `claude --resume <session_id>` replay on startup.
#[derive(Debug, Clone)]
pub struct InterruptedTurn {
    /// `task_messages.id` of the turn's first user row, for logging. The
    /// recovery counter lives on `task_turns` — a turn can hold several user
    /// messages, so bumping one of them would undercount the batch.
    pub task_message_id: i32,
    pub task_id: String,
    /// Backend-tracked agent session ID. Either `session_id` or
    /// `agent_session_id` — they're set together at acknowledgement.
    pub session_id: String,
    pub turn_number: i32,
    pub prompt_text: String,
    pub agent_mode: Option<String>,
    pub recovery_attempts: i32,
}

/// Repository trait for message persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MessageRepository: Send + Sync {
    async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error>;

    async fn append(
        &self,
        msg: &TaskMessage,
        contexts: &[TaskMessageContext],
        content_blocks: &[TaskMessageContentBlock],
    ) -> Result<i32, sqlx::Error>;

    /// Look up a single message by its public uuid. Returns `None` when no
    /// row matches.
    async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TaskMessage>, sqlx::Error>;

    async fn get_pending_user(&self, task_id: &str) -> Result<Vec<TaskMessage>, sqlx::Error>;

    /// Every Task with at least one unacknowledged user message, one row per
    /// Task — the cross-Task scan the reconciler needs (never one Task at a
    /// time), backed by the `task_messages_pending_user_idx` partial index.
    async fn tasks_with_pending_user_messages(
        &self,
    ) -> Result<Vec<PendingUserMessages>, sqlx::Error>;

    async fn get_last_result(&self, task_id: &str) -> Result<Option<TaskMessage>, sqlx::Error>;
    /// Most recent `assistant` row for the task — the agent's final reply when
    /// called from a terminal hook. Used to source the body of the Linear
    /// `response` activity on `agent.completed`.
    async fn get_last_assistant_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error>;
    async fn get_last_user_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error>;

    /// Backfill token usage onto the most recent `assistant` row for the task,
    /// but ONLY when that row has no usage yet (`assistant_usage IS NULL`).
    ///
    /// Codex reports token counts only on the terminal `turn.completed` event —
    /// its assistant message carries none — so unlike Claude the usage never
    /// reaches the per-message columns at insert time. `handle_agent_completed`
    /// calls this to attach the turn's usage to the assistant reply it belongs
    /// to. The `IS NULL` guard makes it a no-op for Claude/tmux (whose assistant
    /// rows already carry inline usage), so it's safe to call for every
    /// transport. Returns `true` when a row was updated.
    async fn backfill_last_assistant_usage<'a>(
        &self,
        task_id: &'a str,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        cache_creation_input_tokens: Option<i32>,
        cache_read_input_tokens: Option<i32>,
        assistant_usage: Option<&'a serde_json::Value>,
    ) -> Result<bool, sqlx::Error>;

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
    /// Mark queued messages as delivered to an agent run, stamping the turn
    /// the run was allocated. Queued rows carry no turn number until here —
    /// a turn is one agent run, and only the ack says which run took them.
    async fn acknowledge_messages(
        &self,
        task_id: &str,
        uuids: &[String],
        session_id: &str,
        turn_number: i32,
    ) -> Result<u64, sqlx::Error>;
    async fn get_last_message_type(
        &self,
        task_id: &str,
    ) -> Result<Option<super::models::MessageType>, sqlx::Error>;

    /// Whether the most recent row for a task is a `Result` whose first text
    /// block is "Stopped by user request". Used by `handle_agent_stopped` to
    /// drop duplicate stop fan-out events before they accumulate phantom rows.
    async fn last_is_stopped_marker(&self, task_id: &str) -> Result<bool, sqlx::Error>;

    /// Whether a turn has already been satisfied by a final assistant reply.
    /// The primary check is scoped to `turn_number`: the most recent row for
    /// that turn must be an `assistant` message with
    /// `assistant_stop_reason = 'end_turn'`.
    ///
    /// The orphaned-turn reaper also needs to recognize the batched-message
    /// shape where the sandbox coalesced multiple pending user messages into
    /// one agent run and all lifecycle/result rows were attributed to the later
    /// sibling turn. In that case an older user-only turn may be considered
    /// completed on the strength of the task's latest later-turn final reply.
    /// Older turns with substantive assistant/system/result rows are not
    /// treated this way; those remain genuine interrupted/orphan candidates.
    ///
    /// The `turn_number` scope matters: `list_unclosed_active_turns` can return
    /// a stale open turn while a *later* turn already has a final reply — without
    /// scoping, that older turn would be wrongly closed as completed.
    async fn last_message_is_completed_reply(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
    ) -> Result<bool, sqlx::Error>;
    async fn get_latest_message_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;

    /// Per-connection inbound/outgoing message counts for the given tasks,
    /// grouped by `(task_id, external_adapter, external_subject_key)`. Only
    /// rows that carry an `external_adapter` (i.e. flowed through an adapter
    /// binding) are counted.
    async fn get_connection_message_counts(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<ConnectionMessageCount>, sqlx::Error>;

    async fn set_external_source_metadata<'a>(
        &self,
        message_uuid: &'a str,
        external_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<bool, sqlx::Error>;

    async fn has_external_event_id(
        &self,
        task_id: &str,
        external_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// Like [`Self::has_external_event_id`] but matches any of the supplied
    /// candidate ids. Used by the dedup gate to recognize both the new
    /// `<provider>:<kind>:<id>` form and any legacy bare-int form for the
    /// same row in a single round-trip.
    async fn has_any_external_event_id(
        &self,
        task_id: &str,
        external_ids: &[String],
    ) -> Result<bool, sqlx::Error>;

    async fn get_external_event_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error>;

    /// Find the owning task for any one of the supplied external IDs.
    ///
    /// Used by GitHub webhook delivery to suppress only the task that authored
    /// a comment or PR event, while still allowing sibling bindings on the
    /// same subject to receive it.
    async fn find_task_id_by_any_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error>;

    /// Keyset-paginated page of messages strictly older than `before_id`
    /// (or the most recent `limit` rows when `before_id` is `None`).
    /// Returned rows are in chronological (oldest-first) order.
    async fn get_paginated(
        &self,
        task_id: &str,
        before_id: Option<i32>,
        limit: i32,
    ) -> Result<MessagePage, sqlx::Error>;

    /// Load exactly the rows needed to re-assemble the top-level Message
    /// affected by a change to `message_uuid`, together with the `uuid` of the
    /// Message the derived upsert should target (the seam's **affected-parent
    /// resolution** and its **dedicated affected-Message loader**).
    ///
    /// When `message_uuid` names a folded row (a `tool_result`/system row
    /// inlined into the assistant Message that called it), the target is that
    /// parent assistant Message, loaded together with all of its `tool_result`
    /// siblings so the transform inlines them. A standalone Message targets
    /// itself. Returns `None` for subagent rows (`parent_uuid` set — those ride
    /// the progress path), when the parent is not resolvable yet, or when the
    /// uuid is unknown.
    async fn get_message_for_assembly(
        &self,
        task_id: &str,
        message_uuid: &str,
    ) -> Result<Option<(MessagePage, String)>, sqlx::Error>;

    /// Aggregate subagent activity for a set of `Agent` tool-use ids: the
    /// per-id message count and summed assistant token usage, computed in one
    /// `GROUP BY parent_uuid`. Used to inject `subagentSummary` onto `Agent`
    /// content blocks for both the read path and the live top-level emit.
    async fn get_subagent_aggregates(
        &self,
        task_id: &str,
        tool_use_ids: &[String],
    ) -> Result<Vec<SubagentAggregate>, sqlx::Error>;

    /// Load a single subagent's rows (those whose `parent_uuid` equals
    /// `tool_use_id`) with their contexts and content blocks, for on-demand
    /// transcript assembly. Returns empty pages when the subagent has no rows.
    async fn get_subagent_messages(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<MessagePage, sqlx::Error>;

    /// Look up a single tool call by its `tool_use_id` within a task. Returns
    /// the assistant's tool_use input plus the matching tool_result content
    /// (if a result has been recorded yet).
    async fn find_tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRow>, sqlx::Error>;

    /// Return whether `file_id` is referenced by any message belonging to
    /// `task_id` via the persisted `attachments.file_ids` JSON. Used by the
    /// sandbox `DownloadUpload` RPC to defense-in-depth the cross-task
    /// scope check.
    async fn file_id_used_in_task(&self, task_id: &str, file_id: &str)
        -> Result<bool, sqlx::Error>;

    /// Locate the most recent user-turn that was acknowledged by the sandbox
    /// (`agent_session_id` set) but never reached a terminal `agent` event
    /// (`completed/error/stopped`). Returns `None` when no such turn exists.
    async fn find_interrupted_turn(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, sqlx::Error>;

    /// Insert a synthetic `Result` row containing `text` at the supplied turn
    /// number to mark an interrupted run as abandoned. Returns the new row id.
    async fn insert_aborted_result(
        &self,
        task_id: &str,
        turn_number: i32,
        text: &str,
    ) -> Result<i32, sqlx::Error>;
}

/// PostgreSQL implementation of [`MessageRepository`].
pub struct PgMessageRepository(pub DbPool);

#[async_trait]
impl MessageRepository for PgMessageRepository {
    async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error> {
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
                agent_mode, model, resume, external_event_id,
                external_adapter, external_source_name, source_task_id,
                start_fresh, sent_by, external_subject_key, assistant_usage
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                $21, $22, $23, $24, $25, $26, $27, $28, $29, $30,
                $31, $32, $33, $34, $35, $36, $37, $38, $39, $40,
                $41
            )
            ON CONFLICT (uuid) DO UPDATE SET
                message_type = EXCLUDED.message_type,
                message_content = EXCLUDED.message_content,
                agent_session_id = COALESCE(EXCLUDED.agent_session_id, task_messages.agent_session_id),
                will_not_resume = COALESCE(EXCLUDED.will_not_resume, task_messages.will_not_resume),
                run_metrics = COALESCE(EXCLUDED.run_metrics, task_messages.run_metrics),
                sdk_metadata = COALESCE(EXCLUDED.sdk_metadata, task_messages.sdk_metadata),
                external_subject_key = COALESCE(EXCLUDED.external_subject_key, task_messages.external_subject_key)
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
        .bind(&msg.external_event_id)
        .bind(&msg.external_adapter)
        .bind(&msg.external_source_name)
        .bind(&msg.source_task_id)
        .bind(msg.start_fresh)
        .bind(&msg.sent_by)
        .bind(&msg.external_subject_key)
        .bind(&msg.assistant_usage)
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

    async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TaskMessage>, sqlx::Error> {
        sqlx::query_as::<_, TaskMessage>(r#"SELECT * FROM task_messages WHERE uuid = $1"#)
            .bind(uuid)
            .fetch_optional(&self.0)
            .await
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

    async fn tasks_with_pending_user_messages(
        &self,
    ) -> Result<Vec<PendingUserMessages>, sqlx::Error> {
        sqlx::query_as::<_, PendingUserMessages>(
            r#"SELECT task_id,
                      MAX(created_at) AS newest_created_at,
                      COUNT(*)        AS pending_count
               FROM task_messages
               WHERE agent_session_id IS NULL AND message_type = 'user'
               GROUP BY task_id"#,
        )
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

    async fn get_last_assistant_message(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskMessage>, sqlx::Error> {
        sqlx::query_as::<_, TaskMessage>(
            r#"SELECT * FROM task_messages
            WHERE task_id = $1 AND message_type = 'assistant'
            ORDER BY timestamp DESC, id DESC
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

    async fn backfill_last_assistant_usage<'a>(
        &self,
        task_id: &'a str,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        cache_creation_input_tokens: Option<i32>,
        cache_read_input_tokens: Option<i32>,
        assistant_usage: Option<&'a serde_json::Value>,
    ) -> Result<bool, sqlx::Error> {
        // Target the single most recent assistant row that still lacks usage,
        // via a subquery so the `IS NULL` guard both selects and gates the write.
        //
        // The `NOT EXISTS` clause confines this to transports that report no
        // per-message usage at all (Codex). Claude reports it inline, but not on
        // every row: one API response can span several rows when the CLI
        // interleaves other traffic between its content-block lines, and the
        // repeats deliberately carry no usage so the tokens are counted once.
        // Without this guard the run's cumulative total lands on one of those
        // repeats and is counted a second time.
        let result = sqlx::query(
            r#"UPDATE task_messages SET
                   assistant_usage_input_tokens = $2,
                   assistant_usage_output_tokens = $3,
                   assistant_usage_cache_creation_input_tokens = $4,
                   assistant_usage_cache_read_input_tokens = $5,
                   assistant_usage = $6
               WHERE id = (
                   SELECT id FROM task_messages
                   WHERE task_id = $1
                     AND message_type = 'assistant'
                     AND assistant_usage IS NULL
                     AND NOT EXISTS (
                         SELECT 1 FROM task_messages prior
                         WHERE prior.task_id = $1
                           AND prior.message_type = 'assistant'
                           AND prior.assistant_usage IS NOT NULL
                     )
                   ORDER BY timestamp DESC, id DESC
                   LIMIT 1
               )"#,
        )
        .bind(task_id)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_creation_input_tokens)
        .bind(cache_read_input_tokens)
        .bind(assistant_usage)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected() > 0)
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
        turn_number: i32,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE task_messages
               SET sent_at = NOW(), session_id = $3, agent_session_id = $3,
                   turn_number = $4
               WHERE task_id = $1
                 AND uuid = ANY($2)
                 AND agent_session_id IS NULL"#,
        )
        .bind(task_id)
        .bind(uuids)
        .bind(session_id)
        .bind(turn_number)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected())
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

    async fn last_message_is_completed_reply(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
    ) -> Result<bool, sqlx::Error> {
        // Scope to the reconciled turn first: a stale open turn must not be
        // closed as completed on the strength of a later turn's final reply
        // when it already has substantive assistant/system/result state of its
        // own (for example, a mid-tool interruption).
        let row: Option<(super::models::MessageType, Option<String>)> = sqlx::query_as(
            r#"SELECT message_type, assistant_stop_reason FROM task_messages
               WHERE task_id = $1
                 AND ($2::int IS NULL OR turn_number = $2)
               ORDER BY timestamp DESC, id DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .bind(turn_number)
        .fetch_optional(&self.0)
        .await?;

        // A final assistant reply ends with stop_reason `end_turn`; a turn cut
        // off mid-tool ends with `tool_use` (or no assistant row at all), which
        // stays a genuine orphan.
        if matches!(
            row,
            Some((super::models::MessageType::Assistant, Some(reason))) if reason == "end_turn"
        ) {
            return Ok(true);
        }

        let Some(turn_number) = turn_number else {
            return Ok(false);
        };

        // Coalesced pending-message run: the orphaned lower turn contains only
        // the inbound user message (plus possible empty assistant placeholders),
        // while the actual final assistant reply was attributed to a later
        // sibling turn by MAX(turn_number)-at-event-arrival resolution.
        let (has_user, has_substantive_non_user, latest_later_completed): (bool, bool, bool) =
            sqlx::query_as(
                r#"SELECT
                     EXISTS (
                       SELECT 1 FROM task_messages
                       WHERE task_id = $1
                         AND turn_number = $2
                         AND message_type = 'user'
                     ) AS has_user,
                     EXISTS (
                       SELECT 1 FROM task_messages
                       WHERE task_id = $1
                         AND turn_number = $2
                         AND message_type <> 'user'
                         AND NOT (
                           message_type = 'assistant'
                           AND assistant_stop_reason IS NULL
                           AND COALESCE(
                             jsonb_array_length(
                               CASE
                                 WHEN jsonb_typeof(message_content) = 'array' THEN message_content
                                 ELSE '[]'::jsonb
                               END
                             ),
                             0
                           ) = 0
                         )
                     ) AS has_substantive_non_user,
                     COALESCE((
                       SELECT message_type = 'assistant'
                          AND assistant_stop_reason = 'end_turn'
                          AND COALESCE(turn_number > $2, false)
                       FROM task_messages
                       WHERE task_id = $1
                       ORDER BY timestamp DESC, id DESC
                       LIMIT 1
                     ), false) AS latest_later_completed"#,
            )
            .bind(task_id)
            .bind(turn_number)
            .fetch_one(&self.0)
            .await?;

        Ok(has_user && !has_substantive_non_user && latest_later_completed)
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

    async fn get_connection_message_counts(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<ConnectionMessageCount>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(vec![]);
        }

        sqlx::query_as::<_, ConnectionMessageCount>(
            r#"SELECT task_id,
                      external_adapter,
                      external_subject_key,
                      COUNT(*) FILTER (WHERE message_role = 'user')      AS incoming,
                      COUNT(*) FILTER (WHERE message_role = 'assistant') AS outgoing
               FROM task_messages
               WHERE task_id = ANY($1) AND external_adapter IS NOT NULL
               GROUP BY task_id, external_adapter, external_subject_key"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await
    }

    async fn set_external_source_metadata<'a>(
        &self,
        message_uuid: &'a str,
        external_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE task_messages
               SET external_event_id = $2,
                   external_adapter    = $3,
                   external_source_name = $4
               WHERE uuid = $1 AND external_event_id IS NULL"#,
        )
        .bind(message_uuid)
        .bind(external_id)
        .bind(provider)
        .bind(source_name)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn has_external_event_id(
        &self,
        task_id: &str,
        external_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let row: (bool,) = sqlx::query_as(
            r#"SELECT EXISTS(
                SELECT 1 FROM task_messages
                WHERE task_id = $1 AND external_event_id = $2
            )"#,
        )
        .bind(task_id)
        .bind(external_id)
        .fetch_one(&self.0)
        .await?;

        Ok(row.0)
    }

    async fn has_any_external_event_id(
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
                WHERE task_id = $1 AND external_event_id = ANY($2)
            )"#,
        )
        .bind(task_id)
        .bind(external_ids)
        .fetch_one(&self.0)
        .await?;

        Ok(row.0)
    }

    async fn get_external_event_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"SELECT external_event_id FROM task_messages
               WHERE task_id = $1 AND external_event_id IS NOT NULL"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn find_task_id_by_any_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error> {
        if external_ids.is_empty() {
            return Ok(None);
        }

        sqlx::query_scalar::<_, String>(
            r#"SELECT task_id
               FROM task_messages
               WHERE external_event_id = ANY($1)
               ORDER BY id DESC
               LIMIT 1"#,
        )
        .bind(external_ids)
        .fetch_optional(&self.0)
        .await
    }

    async fn get_paginated(
        &self,
        task_id: &str,
        before_id: Option<i32>,
        limit: i32,
    ) -> Result<MessagePage, sqlx::Error> {
        let mut messages = sqlx::query_as::<_, TaskMessage>(
            r#"SELECT * FROM task_messages
               WHERE task_id = $1
                 AND parent_uuid IS NULL
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

    async fn get_message_for_assembly(
        &self,
        task_id: &str,
        message_uuid: &str,
    ) -> Result<Option<(MessagePage, String)>, sqlx::Error> {
        // 1. Load the changed row. Subagent rows (parent_uuid set) never upsert
        //    a top-level Message — their activity rides `subagentProgress`.
        let Some(changed) = self.find_by_uuid(message_uuid).await? else {
            return Ok(None);
        };
        if changed.task_id != task_id || changed.parent_uuid.is_some() {
            return Ok(None);
        }

        // 2. Resolve the anchor Message uuid via the shared pure rule: the
        //    parent assistant when the changed row folds (carries tool_result
        //    blocks), else the row itself. Load the candidate parent(s) that
        //    declare a folded tool_use id, then let the seam pick.
        let folded = super::transform::tool_result_ids(&changed.message_content);
        let anchor_uuid = if let Some(tool_use_id) = folded.first() {
            let mut candidates = vec![changed.clone()];
            let parents = sqlx::query_as::<_, TaskMessage>(
                r#"SELECT * FROM task_messages
                   WHERE task_id = $1
                     AND parent_uuid IS NULL
                     AND message_content @> jsonb_build_array(
                           jsonb_build_object('id', $2::text, 'type', 'tool_use')
                         )
                   ORDER BY id DESC
                   LIMIT 1"#,
            )
            .bind(task_id)
            .bind(tool_use_id)
            .fetch_all(&self.0)
            .await?;
            candidates.extend(parents);
            match super::transform::resolve_affected_parent_uuid(&candidates, &changed.uuid) {
                Some(uuid) => uuid,
                // Parent not present yet (or the folded row belongs to a
                // subagent Agent call): the read path hydrates on next open.
                None => return Ok(None),
            }
        } else {
            changed.uuid.clone()
        };

        // 3. Load the anchor row and its declared tool_use ids.
        let Some(anchor) = self.find_by_uuid(&anchor_uuid).await? else {
            return Ok(None);
        };
        let anchor_tool_ids = super::transform::tool_use_ids(&anchor.message_content);

        // 4. Fetch the anchor's folded tool_result siblings (top-level only)
        //    alongside it, so the transform inlines every result present so far.
        let mut messages = vec![anchor];
        if !anchor_tool_ids.is_empty() {
            let siblings = sqlx::query_as::<_, TaskMessage>(
                r#"SELECT m.* FROM task_messages m
                   WHERE m.task_id = $1
                     AND m.parent_uuid IS NULL
                     AND m.uuid <> $2
                     AND EXISTS (
                       SELECT 1 FROM task_message_content_blocks b
                       WHERE b.task_message_id = m.id
                         AND b.tool_use_id = ANY($3)
                     )
                   ORDER BY m.timestamp, m.id"#,
            )
            .bind(task_id)
            .bind(&anchor_uuid)
            .bind(&anchor_tool_ids)
            .fetch_all(&self.0)
            .await?;
            messages.extend(siblings);
        }
        messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.id.cmp(&b.id)));

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

        Ok(Some(((messages, contexts, blocks), anchor_uuid)))
    }

    async fn get_subagent_aggregates(
        &self,
        task_id: &str,
        tool_use_ids: &[String],
    ) -> Result<Vec<SubagentAggregate>, sqlx::Error> {
        if tool_use_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as::<_, SubagentAggregate>(
            r#"SELECT parent_uuid,
                      COUNT(*) AS message_count,
                      COALESCE(SUM(
                        COALESCE(assistant_usage_input_tokens, 0)
                        + COALESCE(assistant_usage_output_tokens, 0)
                      ), 0) AS tokens
               FROM task_messages
               WHERE task_id = $1 AND parent_uuid = ANY($2)
               GROUP BY parent_uuid"#,
        )
        .bind(task_id)
        .bind(tool_use_ids)
        .fetch_all(&self.0)
        .await
    }

    async fn get_subagent_messages(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<MessagePage, sqlx::Error> {
        let messages = sqlx::query_as::<_, TaskMessage>(
            r#"SELECT * FROM task_messages
               WHERE task_id = $1 AND parent_uuid = $2
               ORDER BY timestamp, id"#,
        )
        .bind(task_id)
        .bind(tool_use_id)
        .fetch_all(&self.0)
        .await?;

        if messages.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }

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
        let assistant_row: Option<(serde_json::Value, String)> = sqlx::query_as(
            r#"SELECT message_content, uuid AS parent_message_uuid
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

        let Some((assistant_content, parent_message_uuid)) = assistant_row else {
            return Ok(None);
        };

        let (tool_name, tool_input) = extract_tool_use_block(&assistant_content, tool_use_id)
            .unwrap_or_else(|| {
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
            parent_message_uuid,
        }))
    }

    async fn file_id_used_in_task(
        &self,
        task_id: &str,
        file_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let row: (bool,) = sqlx::query_as(
            r#"SELECT EXISTS(
                SELECT 1 FROM task_messages
                WHERE task_id = $1
                  AND attachments->'file_ids' ? $2
            )"#,
        )
        .bind(task_id)
        .bind(file_id)
        .fetch_one(&self.0)
        .await?;
        Ok(row.0)
    }

    async fn find_interrupted_turn(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, sqlx::Error> {
        // A turn can hold several user messages — everything the sandbox
        // coalesced into one agent run — so this selects the newest turn that
        // never reached a terminal event and then returns *all* of its user
        // rows, oldest first.
        let rows: Vec<(
            i32,
            Option<String>,
            i32,
            serde_json::Value,
            Option<crate::shared_models::AgentMode>,
        )> = sqlx::query_as(
            r#"SELECT m.id, m.agent_session_id, m.turn_number,
                      m.message_content, m.agent_mode
               FROM task_messages m
               WHERE m.task_id = $1
                 AND m.message_type = 'user'
                 AND m.turn_number = (
                   SELECT c.turn_number
                   FROM task_messages c
                   WHERE c.task_id = $1
                     AND c.message_type = 'user'
                     AND c.agent_session_id IS NOT NULL
                     AND c.agent_session_id <> ''
                     AND c.turn_number IS NOT NULL
                     AND NOT EXISTS (
                       SELECT 1 FROM task_events e
                       WHERE e.task_id = c.task_id
                         AND e.turn_number = c.turn_number
                         AND (
                           (e.type = 'agent' AND e.sub_type IN ('completed', 'error', 'stopped'))
                           OR (e.type = 'taskTurn' AND e.sub_type IN ('completed', 'error', 'stopped'))
                           OR (e.type = 'sandbox' AND e.sub_type = 'stopped')
                         )
                     )
                   ORDER BY c.turn_number DESC
                   LIMIT 1
                 )
               ORDER BY m.timestamp, m.id"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        let turn_number = rows[0].2;
        let task_message_id = rows[0].0;
        // Any row of the turn carries the session (they were acknowledged
        // together); take the first that actually has one.
        let Some(session_id) = rows
            .iter()
            .find_map(|(_, sid, _, _, _)| sid.clone().filter(|s| !s.is_empty()))
        else {
            return Ok(None);
        };
        let agent_mode = rows
            .iter()
            .rev()
            .find_map(|(_, _, _, _, mode)| mode.as_ref().map(|m| m.0.clone()));

        // Reproduces what the sandbox dispatcher itself builds from the same
        // rows (`extract_text_from_messages`, joined with newlines over
        // `get_pending_user`'s ordering), so a replay re-sends the same prompt.
        let prompt_text = rows
            .iter()
            .map(|(_, _, _, content, _)| extract_prompt_text(content))
            .collect::<Vec<_>>()
            .join("\n");

        let recovery_attempts: i32 = sqlx::query_scalar(
            "SELECT recovery_attempts FROM task_turns WHERE task_id = $1 AND turn_number = $2",
        )
        .bind(task_id)
        .bind(turn_number)
        .fetch_optional(&self.0)
        .await?
        .unwrap_or(0);

        Ok(Some(InterruptedTurn {
            task_message_id,
            task_id: task_id.to_string(),
            session_id,
            turn_number,
            prompt_text,
            agent_mode,
            recovery_attempts,
        }))
    }

    async fn insert_aborted_result(
        &self,
        task_id: &str,
        turn_number: i32,
        text: &str,
    ) -> Result<i32, sqlx::Error> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let uuid = uuid::Uuid::new_v4().to_string();
        let content = serde_json::json!([{ "type": "text", "text": text }]);

        let row: (i32,) = sqlx::query_as(
            r#"INSERT INTO task_messages (
                task_id, uuid, message_type, user_type, timestamp,
                message_role, message_content, turn_number, will_not_resume
            ) VALUES (
                $1, $2, 'result', 'system', $3,
                'assistant', $4, $5, true
            )
            RETURNING id"#,
        )
        .bind(task_id)
        .bind(&uuid)
        .bind(&now)
        .bind(&content)
        .bind(turn_number)
        .fetch_one(&self.0)
        .await?;
        Ok(row.0)
    }
}

/// Extract the first text block's text from a `message_content` JSON array of
/// the form `[{"type":"text","text":"..."}, ...]`. Falls back to a stringified
/// representation when the shape is unexpected.
fn extract_prompt_text(content: &serde_json::Value) -> String {
    if let Some(arr) = content.as_array() {
        for block in arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    return text.to_string();
                }
            }
        }
    }
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    String::new()
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
        let input = block
            .get("input")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
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

/// Tasks with unacknowledged user messages (convenience wrapper).
pub async fn tasks_with_pending_user_messages(
    pool: &DbPool,
) -> Result<Vec<PendingUserMessages>, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .tasks_with_pending_user_messages()
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

/// Get the last assistant message (convenience wrapper).
pub async fn get_last_assistant_message(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskMessage>, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .get_last_assistant_message(task_id)
        .await
}

/// Backfill usage onto the last assistant message (convenience wrapper).
pub async fn backfill_last_assistant_usage(
    pool: &DbPool,
    task_id: &str,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    cache_creation_input_tokens: Option<i32>,
    cache_read_input_tokens: Option<i32>,
    assistant_usage: Option<&serde_json::Value>,
) -> Result<bool, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .backfill_last_assistant_usage(
            task_id,
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
            assistant_usage,
        )
        .await
}

/// Acknowledge messages with a session_id (convenience wrapper).
pub async fn acknowledge_messages(
    pool: &DbPool,
    task_id: &str,
    uuids: &[String],
    session_id: &str,
    turn_number: i32,
) -> Result<u64, sqlx::Error> {
    PgMessageRepository(pool.clone())
        .acknowledge_messages(task_id, uuids, session_id, turn_number)
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

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;
    use sqlx::PgPool;

    /// Insert one `task_messages` row with explicit pending state and timestamp.
    async fn seed_message(
        pool: &PgPool,
        task_id: &str,
        uuid: &str,
        message_type: &str,
        agent_session_id: Option<&str>,
        created_at: DateTime<Utc>,
    ) {
        sqlx::query(
            r#"INSERT INTO task_messages
                   (task_id, uuid, message_type, user_type, timestamp, message_role,
                    message_content, agent_session_id, created_at)
               VALUES ($1, $2, $3, 'human', $4, 'user', '{}'::jsonb, $5, $6)"#,
        )
        .bind(task_id)
        .bind(uuid)
        .bind(message_type)
        .bind(created_at.to_rfc3339())
        .bind(agent_session_id)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("seed message");
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid ts")
    }

    #[sqlx::test]
    async fn pending_scan_returns_one_row_per_task_with_newest_and_count(pool: PgPool) {
        // Task A: two pending user messages — newest wins, count is 2.
        seed_message(&pool, "a", "a1", "user", None, at(1_000)).await;
        seed_message(&pool, "a", "a2", "user", None, at(2_000)).await;
        // Task A also has an acknowledged user message and an assistant message —
        // neither is pending, so neither inflates the count nor the timestamp.
        seed_message(&pool, "a", "a3", "user", Some("sess-1"), at(3_000)).await;
        seed_message(&pool, "a", "a4", "assistant", None, at(4_000)).await;
        // Task B: one pending user message.
        seed_message(&pool, "b", "b1", "user", None, at(500)).await;
        // Task C: only an acknowledged message — never appears.
        seed_message(&pool, "c", "c1", "user", Some("sess-2"), at(9_000)).await;

        let mut rows = tasks_with_pending_user_messages(&pool).await.unwrap();
        rows.sort_by(|l, r| l.task_id.cmp(&r.task_id));

        assert_eq!(rows.len(), 2, "only tasks with a pending user message appear");
        assert_eq!(rows[0].task_id, "a");
        assert_eq!(rows[0].newest_created_at, at(2_000), "newest pending, not the acked one");
        assert_eq!(rows[0].pending_count, 2);
        assert_eq!(rows[1].task_id, "b");
        assert_eq!(rows[1].newest_created_at, at(500));
        assert_eq!(rows[1].pending_count, 1);
    }
}
