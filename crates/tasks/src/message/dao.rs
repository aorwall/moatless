//! Message DAO — queries for task message data access.
//!
//! `mockall::automock` mirrors every trait signature below into a mock whose
//! matcher parameters trip lints on argument shapes we chose deliberately. The
//! generated item lands in this module, so this is the only scope that can
//! carry the suppression.
#![cfg_attr(
    test,
    expect(
        clippy::ref_option_ref,
        reason = "mockall turns each `Option<&T>` argument into a `&Option<&T>` matcher"
    )
)]

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::{MessageType, TaskMessage, TaskMessageContext};
use crate::task::dao as task_dao;
use crate::task_activities;
use crate::task_activities::dao::{self as activities, Activity};
use crate::task_activities::mapping::{ActivityKind, ConversationWrite};
use moatless_core::db::DbPool;

/// The tool calls that hand a conversation back to a person. A sidebar shows
/// whether one is outstanding, so the message that raises one moves a listing
/// row even though the agent wrote it.
///
/// Keep in sync with `handlers::INTERACTIVE_TOOL_NAMES`,
/// `transform::check_pending_user_input` and the same pair in `moatless_t3code::dao`,
/// which is what reads the flag back out.
const INTERACTIVE_TOOLS: &[&str] = &["AskUserQuestion", "ExitPlanMode"];

/// Whether appending this message moves anything a personal sidebar shows.
///
/// Two things, and deliberately not a third. A message **a person wrote** moves
/// the activity timestamp, which is what T3 orders the sidebar by — and the
/// answer to a question arrives as one of these, so answering is covered here
/// too. A message **raising an interactive tool call** flips the
/// waiting-on-a-person flag, and that one the agent writes.
///
/// Everything else the agent produces is excluded, and that exclusion is what
/// keeps this design affordable: ordinary agent output is the highest-rate path
/// in the system, the only sidebar value it could touch is the activity
/// timestamp, and T3 defines that as counting messages authored by a person. A
/// tool result is *not* treated as an answer here for the same reason — every
/// tool the agent runs produces one, and the answers that matter are the
/// person's own `user` rows.
fn moves_a_listing_row(msg: &TaskMessage, conversation: &ConversationWrite) -> bool {
    if msg.message_type == MessageType::User {
        return true;
    }
    // Read off the activities this write creates rather than out of a jsonb
    // document. Same question, asked of the rows that now answer it.
    conversation.activities.iter().any(|activity| {
        activity.kind == ActivityKind::ToolCall
            && INTERACTIVE_TOOLS.contains(&activity.summary.as_str())
    })
}

/// The next position inside one subagent's own transcript.
///
/// A subagent's activities never enter the Task's stream — they are assembled on
/// demand behind the `Agent` call that spawned them — so they take nothing from
/// the Task's counter. Within their own transcript they still need an order, and
/// arrival order is the order they happened in.
///
/// A redelivered block does not move: `activities::upsert` deliberately leaves
/// `position` alone. Two blocks racing can take the same number, which the
/// reads break on `activity_id` — the block's own index, so the tie resolves the
/// way the source ordered it.
async fn next_subagent_position(
    tx: &mut sqlx::PgConnection,
    task_id: &str,
    parent_uuid: &str,
) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"SELECT COALESCE(MAX(position), 0) + 1 FROM task_activities
           WHERE task_id = $1 AND parent_uuid = $2"#,
    )
    .bind(task_id)
    .bind(parent_uuid)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// A page of Messages with their context rows and the activities that happened
/// on them — the two ordered collections a conversation is, plus the context
/// rows the frontend attaches to a user Message.
pub type MessagePage = (Vec<TaskMessage>, Vec<TaskMessageContext>, Vec<Activity>);

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
/// against the Sandbox row's `desired_state_updated_at` to decide whether a wake
/// supersedes a later stop/remove.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingUserMessages {
    pub task_id: String,
    pub newest_created_at: DateTime<Utc>,
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
    /// Reasoning effort the interrupted turn ran with, so the replay repeats it.
    pub effort: Option<String>,
    pub recovery_attempts: i32,
}

/// DAO trait for message persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MessageDao: Send + Sync {
    async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error>;

    /// Write a Message and everything its content array changed, in one
    /// transaction. A reader never sees one without the other.
    async fn append(
        &self,
        msg: &TaskMessage,
        contexts: &[TaskMessageContext],
        conversation: &ConversationWrite,
    ) -> Result<i32, sqlx::Error>;

    /// Write what a turn *did* when the turn said nothing.
    ///
    /// A Message row exists when somebody said something, so a turn that only
    /// called tools — and a carrier row that only relays a result — has none to
    /// hang its activities off. `message_uuid` is still what the conversation
    /// change names: for the tool-result case it is the calling Message, which
    /// is the row a reader will see move.
    async fn append_activities(
        &self,
        task_id: &str,
        message_uuid: &str,
        conversation: &ConversationWrite,
    ) -> Result<(), sqlx::Error>;

    /// Write a Sandbox status change or error as an activity, unless it restates
    /// the one before it. Returns whether a row was written.
    async fn record_sandbox_activity(
        &self,
        activity: &crate::task_activities::mapping::ActivityRow,
    ) -> Result<bool, sqlx::Error>;

    /// Write an activity that happened on no Message, in a change of its own.
    /// Returns whether a row was written — `false` where the event that carries
    /// it was redelivered.
    async fn record_standalone_activity(
        &self,
        activity: &crate::task_activities::mapping::ActivityRow,
    ) -> Result<bool, sqlx::Error>;

    /// The context rows attached to a set of Messages, keyed by
    /// `task_message_id`.
    ///
    /// The preview URL and the selected element a person sent with a message.
    /// They are persisted once, on the way in, and rebuilt into the agent's view
    /// of the message on the way out — which is what this read is for.
    async fn get_contexts_for_messages(
        &self,
        task_id: &str,
        message_ids: &[i32],
    ) -> Result<HashMap<i32, Vec<TaskMessageContext>>, sqlx::Error>;

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

    /// Claim, on the call that made it, what that call created on the other
    /// side. The Message twin of this writes to a row that a turn which only ran
    /// the command never created; the call always has one.
    async fn claim_activity_external_source<'a>(
        &self,
        task_id: &'a str,
        tool_use_id: &'a str,
        external_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<bool, sqlx::Error>;

    /// Record an external event this Task produced.
    ///
    /// The entry for a caller with no `tool_use_id` to hang the record on — a
    /// `moat` command in a sandbox shell. See
    /// [`outbound_events`](crate::task::outbound_events).
    async fn record_external_event<'a>(
        &self,
        task_id: &'a str,
        external_event_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<(), sqlx::Error>;

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

    /// Load exactly the rows needed to re-assemble the Message affected by a
    /// change to `message_uuid`, together with the `uuid` the derived upsert
    /// should target.
    ///
    /// The named uuid *is* the target — no "affected parent" is resolved here. A
    /// result settles the call's own activity, and the writer resolves the anchor
    /// before it commits, so a change naming a carrier row is redirected once, on
    /// the way in.
    ///
    /// Returns `None` for a subagent row (`parent_uuid` set — those ride the
    /// progress path), and for a uuid this Task has neither text nor activity
    /// under.
    async fn get_message_for_assembly(
        &self,
        task_id: &str,
        message_uuid: &str,
    ) -> Result<Option<(MessagePage, String)>, sqlx::Error>;

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
}

/// PostgreSQL implementation of [`MessageDao`].
pub struct PgMessageDao(pub DbPool);

#[async_trait]
impl MessageDao for PgMessageDao {
    async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error> {
        let (messages, contexts, activities) = tokio::try_join!(
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
            task_activities::dao::for_task(&self.0, task_id),
        )?;

        Ok((messages, contexts, activities))
    }

    async fn append(
        &self,
        msg: &TaskMessage,
        contexts: &[TaskMessageContext],
        conversation: &ConversationWrite,
    ) -> Result<i32, sqlx::Error> {
        let mut tx = self.0.begin().await?;

        let row: (i32,) = sqlx::query_as(
            r#"INSERT INTO task_messages (
                task_id, uuid, message_type, user_type, parent_uuid,
                is_sidechain, cwd, session_id, turn_number, version,
                git_branch, request_id, timestamp, message_role, message_text,
                assistant_message_id, assistant_message_type, assistant_message_model,
                assistant_stop_reason, assistant_stop_sequence,
                assistant_usage_input_tokens, assistant_usage_output_tokens,
                assistant_usage_cache_creation_input_tokens,
                assistant_usage_cache_read_input_tokens,
                tool_use_result, sdk_metadata, attachments, agent_session_id,
                will_not_resume, run_metrics,
                agent_mode, model, resume, external_event_id,
                external_adapter, external_source_name, source_task_id,
                start_fresh, sent_by, external_subject_key, assistant_usage,
                tool_response, effort, external_author, skills
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                $21, $22, $23, $24, $25, $26, $27, $28, $29, $30,
                $31, $32, $33, $34, $35, $36, $37, $38, $39, $40,
                $41, $42, $43, $44, $45
            )
            ON CONFLICT (uuid) DO UPDATE SET
                message_type = EXCLUDED.message_type,
                -- Text is spliced below, at each block's own index. A whole
                -- string arriving here belongs to a caller that wrote its
                -- Message in one piece, and must not blank out a Message the
                -- per-block path is still assembling.
                message_text = COALESCE(EXCLUDED.message_text, task_messages.message_text),
                agent_session_id = COALESCE(EXCLUDED.agent_session_id, task_messages.agent_session_id),
                will_not_resume = COALESCE(EXCLUDED.will_not_resume, task_messages.will_not_resume),
                run_metrics = COALESCE(EXCLUDED.run_metrics, task_messages.run_metrics),
                sdk_metadata = COALESCE(EXCLUDED.sdk_metadata, task_messages.sdk_metadata),
                external_subject_key = COALESCE(EXCLUDED.external_subject_key, task_messages.external_subject_key),
                -- Usage is admitted exactly once per assembled Message: a CLI that
                -- repeats usage on every per-block source event keeps the first
                -- non-null value, so re-upserting blocks never double-counts cost.
                assistant_usage_input_tokens = COALESCE(task_messages.assistant_usage_input_tokens, EXCLUDED.assistant_usage_input_tokens),
                assistant_usage_output_tokens = COALESCE(task_messages.assistant_usage_output_tokens, EXCLUDED.assistant_usage_output_tokens),
                assistant_usage_cache_creation_input_tokens = COALESCE(task_messages.assistant_usage_cache_creation_input_tokens, EXCLUDED.assistant_usage_cache_creation_input_tokens),
                assistant_usage_cache_read_input_tokens = COALESCE(task_messages.assistant_usage_cache_read_input_tokens, EXCLUDED.assistant_usage_cache_read_input_tokens),
                assistant_usage = COALESCE(task_messages.assistant_usage, EXCLUDED.assistant_usage),
                tool_response = COALESCE(EXCLUDED.tool_response, task_messages.tool_response)
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
        .bind(&msg.message_text)
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
        .bind(&msg.tool_response)
        .bind(&msg.effort)
        .bind(&msg.external_author)
        // Deliberately absent from the DO UPDATE SET above: that path is the
        // per-block agent-message merge, and must not blank a user row's skills.
        .bind(&msg.skills)
        .fetch_one(&mut *tx)
        .await?;

        let message_id = row.0;

        // Child rows are re-derived from the assembled content, so replace them
        // rather than append. Replacement is scoped to the child set the caller
        // actually supplies: per-block assembly always passes the full assembled
        // set (so redelivery stays idempotent — no accumulating duplicate rows),
        // while callers that manage no children pass an empty slice and must not
        // have existing rows deleted out from under them on an upsert.
        if !contexts.is_empty() {
            sqlx::query("DELETE FROM task_message_contexts WHERE task_message_id = $1")
                .bind(message_id)
                .execute(&mut *tx)
                .await?;
        }

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

        // Text, spliced at each block's own index.
        //
        // `message_text_parts[i]` is subscripted assignment, which enlarges the
        // array — and creates it when the column is NULL — so an out-of-order or
        // redelivered text block lands in the same slot either way. That is the
        // idempotency the jsonb splice this replaces used to provide, and it is
        // why an append would not do. PostgreSQL subscripts from 1.
        for part in &conversation.text {
            sqlx::query("UPDATE task_messages SET message_text_parts[$2] = $3 WHERE id = $1")
                .bind(message_id)
                .bind(i32::try_from(part.block_index).unwrap_or(i32::MAX) + 1)
                .bind(&part.text)
                .execute(&mut *tx)
                .await?;
        }
        if !conversation.text.is_empty() {
            // `array_to_string` skips NULL holes, which is what the slots for
            // blocks that are not text are. The separator is the blank line
            // markdown needs between paragraphs — what `push_paragraph` and the
            // per-block render this replaces both produced.
            sqlx::query(
                r#"UPDATE task_messages
                      SET message_text = array_to_string(message_text_parts, E'\n\n')
                    WHERE id = $1"#,
            )
            .bind(message_id)
            .execute(&mut *tx)
            .await?;
        }

        // The conversation's position, allocated once for the Message and every
        // activity that happened on it, so they interleave in one stream.
        let position = match msg.parent_uuid.as_deref() {
            None => {
                moatless_backend_core::change_feed::conversation::record_message_change(
                    &mut tx,
                    &msg.task_id,
                    &msg.uuid,
                )
                .await?
            }
            // A subagent's rows are never in the Task's own stream, so they
            // allocate nothing from its counter — see the note below on why they
            // record no change. They still need a position to sort by within
            // their own transcript.
            Some(parent_uuid) => next_subagent_position(&mut tx, &msg.task_id, parent_uuid).await?,
        };

        for activity in &conversation.activities {
            task_activities::dao::upsert(&mut tx, position, activity).await?;
        }
        for outcome in &conversation.outcomes {
            task_activities::dao::settle(&mut tx, outcome).await?;
        }
        if let Some(usage) = conversation.usage {
            task_activities::dao::admit_usage(
                &mut tx,
                &msg.task_id,
                &msg.uuid,
                msg.parent_uuid.as_deref(),
                usage,
            )
            .await?;
        }

        // The position above *is* that record. Every write to a Message a
        // conversation shows is a change to that conversation — including the
        // in-place merges that assemble one Message from its blocks, which move
        // no row count and no id and are otherwise undetectable from outside
        // this transaction.
        //
        // Subagent rows are excluded because no transcript reads them: they are
        // assembled on demand behind the tool call that spawned them, and they
        // outnumber the rows that are rendered. Recording them would spend a
        // conversation's replay window on positions that project to nothing.
        //
        // Before the shell's allocator and after every `task_messages` write,
        // per `record_message_change`'s ordering rule and `record_change`'s
        // "call this last".

        // A person writing, or the agent asking, is a listing change. Last,
        // after every `task_messages` write above — `record_change`'s "call this
        // last", and this is the busiest transaction that allocates.
        if moves_a_listing_row(msg, conversation) {
            task_dao::clear_settled_for_all_followers(&mut tx, &msg.task_id).await?;
            moatless_backend_core::change_feed::listing::record_change(&mut tx, &msg.task_id)
                .await?;
        }

        tx.commit().await?;
        Ok(message_id)
    }

    async fn append_activities(
        &self,
        task_id: &str,
        message_uuid: &str,
        conversation: &ConversationWrite,
    ) -> Result<(), sqlx::Error> {
        if conversation.is_empty() {
            return Ok(());
        }
        let mut tx = self.0.begin().await?;

        // A subagent's activities are never in the Task's own stream, so they
        // allocate nothing from its counter — the same exclusion `append` makes,
        // and for the same reason: they are assembled on demand behind the call
        // that spawned them. They still need a position to sort by within their
        // own transcript, and arrival order is the order they happened in.
        let parent_uuid = conversation
            .activities
            .iter()
            .find_map(|activity| activity.parent_uuid.as_deref());
        let position = match parent_uuid {
            Some(parent_uuid) => next_subagent_position(&mut tx, task_id, parent_uuid).await?,
            None => {
                moatless_backend_core::change_feed::conversation::record_message_change(
                    &mut tx,
                    task_id,
                    message_uuid,
                )
                .await?
            }
        };

        for activity in &conversation.activities {
            task_activities::dao::upsert(&mut tx, position, activity).await?;
        }
        for outcome in &conversation.outcomes {
            task_activities::dao::settle(&mut tx, outcome).await?;
        }
        if let Some(usage) = conversation.usage {
            task_activities::dao::admit_usage(&mut tx, task_id, message_uuid, parent_uuid, usage)
                .await?;
        }

        if conversation.activities.iter().any(|activity| {
            activity.kind == ActivityKind::ToolCall
                && INTERACTIVE_TOOLS.contains(&activity.summary.as_str())
        }) {
            moatless_backend_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn record_sandbox_activity(
        &self,
        activity: &crate::task_activities::mapping::ActivityRow,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        // The collapse check and the allocation both live in `record_sandbox`:
        // a restated status must spend no position and record no change, so the
        // change that names this activity is allocated only once the row is
        // known to be worth writing.
        let written = task_activities::dao::record_sandbox(&mut tx, activity).await?;
        tx.commit().await?;
        Ok(written)
    }

    async fn record_standalone_activity(
        &self,
        activity: &crate::task_activities::mapping::ActivityRow,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        // `record_activity_change`, not `record_session_change`: the change names
        // this activity, so a live reader re-reads the one row rather than the
        // conversation's own session, which nothing about the reading moved.
        let position = moatless_backend_core::change_feed::conversation::record_activity_change(
            &mut tx,
            &activity.task_id,
            &activity.activity_id,
        )
        .await?;
        let written = task_activities::dao::record_standalone(&mut tx, position, activity).await?;
        tx.commit().await?;
        Ok(written)
    }

    async fn get_contexts_for_messages(
        &self,
        task_id: &str,
        message_ids: &[i32],
    ) -> Result<HashMap<i32, Vec<TaskMessageContext>>, sqlx::Error> {
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query_as::<_, TaskMessageContext>(
            r#"SELECT id, task_message_id, type, reference_id, data, created_at
               FROM task_message_contexts
               WHERE task_id = $1 AND task_message_id = ANY($2)
               ORDER BY id"#,
        )
        .bind(task_id)
        .bind(message_ids)
        .fetch_all(&self.0)
        .await?;

        let mut by_message: HashMap<i32, Vec<TaskMessageContext>> = HashMap::new();
        for row in rows {
            by_message.entry(row.task_message_id).or_default().push(row);
        }
        Ok(by_message)
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
            r#"SELECT task_id, MAX(created_at) AS newest_created_at
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
                         -- An empty assistant placeholder: a row that stopped
                         -- for no stated reason and said nothing. Text is the
                         -- row itself, so an empty one is a row with no text
                         -- on it.
                         AND NOT (
                           message_type = 'assistant'
                           AND assistant_stop_reason IS NULL
                           AND COALESCE(message_text, '') = ''
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

    async fn claim_activity_external_source<'a>(
        &self,
        task_id: &'a str,
        tool_use_id: &'a str,
        external_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<bool, sqlx::Error> {
        activities::claim_external(
            &self.0,
            task_id,
            tool_use_id,
            external_id,
            provider,
            source_name,
        )
        .await
    }

    async fn record_external_event<'a>(
        &self,
        task_id: &'a str,
        external_event_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<(), sqlx::Error> {
        crate::task::outbound_events::record(
            &self.0,
            task_id,
            external_event_id,
            provider,
            source_name,
        )
        .await
    }

    async fn has_external_event_id(
        &self,
        task_id: &str,
        external_id: &str,
    ) -> Result<bool, sqlx::Error> {
        self.has_any_external_event_id(task_id, std::slice::from_ref(&external_id.to_string()))
            .await
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

        // An id claimed by a call rather than by a Message: an outbound comment
        // whose turn said nothing and therefore wrote no Message row.
        if row.0 || activities::claims_any_external(&self.0, task_id, external_ids).await? {
            return Ok(true);
        }
        // An id claimed by a `moat` command, which has no tool-use id to hang
        // the activity tag on.
        crate::task::outbound_events::has_any(&self.0, task_id, external_ids).await
    }

    async fn get_external_event_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
        let mut ids: Vec<String> = sqlx::query_scalar(
            r#"SELECT external_event_id FROM task_messages
               WHERE task_id = $1 AND external_event_id IS NOT NULL"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await?;

        ids.extend(activities::external_claims(&self.0, task_id).await?);
        Ok(ids)
    }

    async fn find_task_id_by_any_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error> {
        if external_ids.is_empty() {
            return Ok(None);
        }

        let by_message = sqlx::query_scalar::<_, String>(
            r#"SELECT task_id
               FROM task_messages
               WHERE external_event_id = ANY($1)
               ORDER BY id DESC
               LIMIT 1"#,
        )
        .bind(external_ids)
        .fetch_optional(&self.0)
        .await?;

        if let Some(task_id) = by_message {
            return Ok(Some(task_id));
        }
        if let Some(task_id) = activities::task_claiming_external(&self.0, external_ids).await? {
            return Ok(Some(task_id));
        }
        crate::task::outbound_events::task_for_any(&self.0, external_ids).await
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

        // Activities are fetched by when they happened, not by the Messages
        // named in this page. A turn that only called tools said nothing and has
        // no Message row to be named by, and its calls are exactly what this
        // page has to show — so the window is "everything from the oldest
        // Message on this page onwards".
        let since = messages.first().and_then(|m| m.created_at);

        let (contexts, activities) = tokio::try_join!(
            sqlx::query_as::<_, TaskMessageContext>(
                r#"SELECT id, task_message_id, type, reference_id, data, created_at
                   FROM task_message_contexts
                   WHERE task_id = $1 AND task_message_id = ANY($2)
                   ORDER BY id"#,
            )
            .bind(task_id)
            .bind(&message_ids)
            .fetch_all(&self.0),
            task_activities::dao::for_task_since(&self.0, task_id, since),
        )?;

        Ok((messages, contexts, activities))
    }

    async fn get_message_for_assembly(
        &self,
        task_id: &str,
        message_uuid: &str,
    ) -> Result<Option<(MessagePage, String)>, sqlx::Error> {
        let changed = self.find_by_uuid(message_uuid).await?;

        // A subagent's rows never upsert a top-level Message: its progress rides
        // `subagentProgress` instead.
        if let Some(row) = &changed {
            if row.task_id != task_id || row.parent_uuid.is_some() {
                return Ok(None);
            }
        }

        let message_ids: Vec<i32> = changed.iter().map(|m| m.id).collect();
        let named = [message_uuid.to_string()];
        let (contexts, activities) = tokio::try_join!(
            sqlx::query_as::<_, TaskMessageContext>(
                r#"SELECT id, task_message_id, type, reference_id, data, created_at
                   FROM task_message_contexts
                   WHERE task_id = $1 AND task_message_id = ANY($2)
                   ORDER BY id"#,
            )
            .bind(task_id)
            .bind(&message_ids)
            .fetch_all(&self.0),
            task_activities::dao::for_messages(&self.0, task_id, &named),
        )?;

        // Neither text nor activity under this uuid: a change naming a Message
        // this Task does not have.
        if changed.is_none() && activities.is_empty() {
            return Ok(None);
        }

        let messages: Vec<TaskMessage> = changed.into_iter().collect();
        Ok(Some((
            (messages, contexts, activities),
            message_uuid.to_string(),
        )))
    }

    async fn get_subagent_aggregates(
        &self,
        task_id: &str,
        tool_use_ids: &[String],
    ) -> Result<Vec<SubagentAggregate>, sqlx::Error> {
        if tool_use_ids.is_empty() {
            return Ok(vec![]);
        }
        // Over `task_message_usage`, not `task_messages`. The tokens a subagent
        // spent are reported almost entirely on turns that only called tools —
        // turns that said nothing and so have no Message row to sum. Usage is
        // admitted against the Message's uuid instead, which is what makes the
        // count survive the row it was reported on.
        sqlx::query_as::<_, SubagentAggregate>(
            r#"SELECT parent_uuid,
                      COUNT(*) AS message_count,
                      COALESCE(SUM(input_tokens + output_tokens), 0) AS tokens
               FROM task_message_usage
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
        let (contexts, activities) = tokio::try_join!(
            sqlx::query_as::<_, TaskMessageContext>(
                r#"SELECT id, task_message_id, type, reference_id, data, created_at
                   FROM task_message_contexts
                   WHERE task_id = $1 AND task_message_id = ANY($2)
                   ORDER BY id"#,
            )
            .bind(task_id)
            .bind(&message_ids)
            .fetch_all(&self.0),
            task_activities::dao::for_subagent(&self.0, task_id, tool_use_id),
        )?;

        Ok((messages, contexts, activities))
    }

    async fn find_tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRow>, sqlx::Error> {
        // One row, found by an index: the call and its outcome are the same row,
        // so there is no second lookup for whichever later Message carried the
        // result.
        let Some(activity) =
            task_activities::dao::find_tool_call(&self.0, task_id, tool_use_id).await?
        else {
            return Ok(None);
        };

        Ok(Some(ToolCallRow {
            tool_use_id: tool_use_id.to_string(),
            tool_name: activity.summary.clone(),
            tool_input: activity.tool_input().clone(),
            result_content: activity
                .result()
                .and_then(crate::task_activities::mapping::result_text)
                .map(std::string::ToString::to_string),
            // `None` while the call is still out: the caller distinguishes "came
            // back clean" from "has not come back", and a bare `false` would
            // report the second as the first.
            is_error: activity.is_settled().then_some(activity.is_error),
            tool_use_result: activity.typed_result().cloned(),
            parent_message_uuid: activity.message_uuid.clone().unwrap_or_default(),
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
            Option<String>,
            Option<moatless_core::shared_models::AgentMode>,
            Option<String>,
        )> = sqlx::query_as(
            r#"SELECT m.id, m.agent_session_id, m.turn_number,
                      m.message_text, m.agent_mode, m.effort
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
            .find_map(|(_, sid, _, _, _, _)| sid.clone().filter(|s| !s.is_empty()))
        else {
            return Ok(None);
        };
        let agent_mode = rows
            .iter()
            .rev()
            .find_map(|(_, _, _, _, mode, _)| mode.as_ref().map(|m| m.0.clone()));
        let effort = rows
            .iter()
            .rev()
            .find_map(|(_, _, _, _, _, effort)| effort.clone());

        // Reproduces what the sandbox dispatcher itself builds from the same
        // rows (`extract_text_from_messages`, joined with newlines over
        // `get_pending_user`'s ordering), so a replay re-sends the same prompt.
        let prompt_text = rows
            .iter()
            .map(|(_, _, _, text, _, _)| text.clone().unwrap_or_default())
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
            effort,
            recovery_attempts,
        }))
    }
}

// =========================================================================
// Free-function wrappers for integration tests
// =========================================================================

/// Append a Message with its contexts and everything its content changed
/// (convenience wrapper).
pub async fn append(
    pool: &DbPool,
    msg: &TaskMessage,
    contexts: &[TaskMessageContext],
    conversation: &ConversationWrite,
) -> Result<i32, sqlx::Error> {
    PgMessageDao(pool.clone())
        .append(msg, contexts, conversation)
        .await
}

/// Every Message, context row and activity of a Task (convenience wrapper).
pub async fn get_all(pool: &DbPool, task_id: &str) -> Result<MessagePage, sqlx::Error> {
    PgMessageDao(pool.clone()).get_all(task_id).await
}

/// Get pending user messages (convenience wrapper).
pub async fn get_pending_user(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<TaskMessage>, sqlx::Error> {
    PgMessageDao(pool.clone()).get_pending_user(task_id).await
}

/// Tasks with unacknowledged user messages (convenience wrapper).
pub async fn tasks_with_pending_user_messages(
    pool: &DbPool,
) -> Result<Vec<PendingUserMessages>, sqlx::Error> {
    PgMessageDao(pool.clone())
        .tasks_with_pending_user_messages()
        .await
}

/// Set session_id for all pending messages (convenience wrapper).
pub async fn set_session_id_for_task(
    pool: &DbPool,
    task_id: &str,
    session_id: &str,
) -> Result<u64, sqlx::Error> {
    PgMessageDao(pool.clone())
        .set_session_id_for_task(task_id, session_id)
        .await
}

/// Get the last result message (convenience wrapper).
pub async fn get_last_result(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskMessage>, sqlx::Error> {
    PgMessageDao(pool.clone()).get_last_result(task_id).await
}

/// Get the last assistant message (convenience wrapper).
pub async fn get_last_assistant_message(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskMessage>, sqlx::Error> {
    PgMessageDao(pool.clone())
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
    PgMessageDao(pool.clone())
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
    PgMessageDao(pool.clone())
        .acknowledge_messages(task_id, uuids, session_id, turn_number)
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
                    agent_session_id, created_at)
               VALUES ($1, $2, $3, 'human', $4, 'user', $5, $6)"#,
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
    async fn pending_scan_returns_one_row_per_task_with_newest_pending(pool: PgPool) {
        // Task A: two pending user messages — newest wins.
        seed_message(&pool, "a", "a1", "user", None, at(1_000)).await;
        seed_message(&pool, "a", "a2", "user", None, at(2_000)).await;
        // Task A also has an acknowledged user message and an assistant message —
        // neither is pending, so neither moves the newest-pending timestamp.
        seed_message(&pool, "a", "a3", "user", Some("sess-1"), at(3_000)).await;
        seed_message(&pool, "a", "a4", "assistant", None, at(4_000)).await;
        // Task B: one pending user message.
        seed_message(&pool, "b", "b1", "user", None, at(500)).await;
        // Task C: only an acknowledged message — never appears.
        seed_message(&pool, "c", "c1", "user", Some("sess-2"), at(9_000)).await;

        let mut rows = tasks_with_pending_user_messages(&pool).await.unwrap();
        rows.sort_by(|l, r| l.task_id.cmp(&r.task_id));

        assert_eq!(
            rows.len(),
            2,
            "only tasks with a pending user message appear"
        );
        assert_eq!(rows[0].task_id, "a");
        assert_eq!(
            rows[0].newest_created_at,
            at(2_000),
            "newest pending, not the acked one"
        );
        assert_eq!(rows[1].task_id, "b");
        assert_eq!(rows[1].newest_created_at, at(500));
    }
}
