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

use super::models::{
    InterruptedTurn, MessagePage, MessageType, TaskMessage, TaskMessageContext, ToolCallRow,
};
use crate::task::dao as task_dao;
use crate::task_activities;
use crate::task_activities::mapping::{ActivityKind, ConversationWrite};
use moatless_core::db::DbPool;
use moatless_core::db_enum::decode_required;
use moatless_ports::task::EventSubject;

/// Keep in sync with `handlers::INTERACTIVE_TOOL_NAMES`,
/// `transform::check_pending_user_input`, and `moatless_t3code::dao`'s pair. A
/// message raising one of these moves a sidebar listing row.
const INTERACTIVE_TOOLS: &[&str] = &["AskUserQuestion", "ExitPlanMode"];

/// Whether this message moves anything a sidebar shows: a person-authored
/// message moves the activity timestamp T3 orders by; raising an interactive
/// tool call flips the waiting-on-a-person flag.
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

/// The next position inside one subagent's own transcript; subagent activities
/// never touch the Task's counter. A redelivered block does not move; a race
/// breaks its tie on `activity_id`.
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

/// Per-`Agent`-call subagent aggregate: message count + summed assistant
/// tokens, keyed by the spawning `Agent` tool-use id (`parent_uuid`). Feeds the
/// `subagentSummary` injected onto the top-level read path and live emit.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct SubagentAggregate {
    /// The spawning `Agent` tool-use id (`task_messages.parent_uuid`).
    pub parent_uuid: String,
    /// `COUNT(*)` of the subagent's rows.
    pub message_count: i64,
    /// `SUM(input_tokens + output_tokens)` across the subagent's assistant
    /// rows.
    pub tokens: i64,
}

/// DAO trait for message persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait MessageDao: Send + Sync {
    async fn get_all(&self, task_id: &str) -> Result<MessagePage, sqlx::Error>;

    /// Write a Message and everything its content array changed, in one
    /// transaction. A reader never sees one without the other.
    async fn append(
        &self,
        msg: &TaskMessage,
        contexts: &[TaskMessageContext],
        conversation: &ConversationWrite,
    ) -> Result<i32, sqlx::Error>;

    /// Record a backend failure as a `task.error` event and project it as an
    /// Error Message, in one transaction.
    async fn append_error_result(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
        error_text: &str,
    ) -> Result<(), sqlx::Error>;

    /// Write what a turn *did* when it said nothing: a tool-only turn has no
    /// Message row of its own. `message_uuid` still names the conversation
    /// change — the calling Message, for a tool result.
    async fn append_activities(
        &self,
        task_id: &str,
        message_uuid: &str,
        conversation: &ConversationWrite,
    ) -> Result<(), sqlx::Error>;

    /// Write a Sandbox status change or error as an activity, unless it
    /// restates the one before it. Returns whether a row was written.
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
    /// `task_message_id`: the preview URL and selected element a person sent,
    /// persisted once and rebuilt into the agent's view on the way out.
    async fn get_contexts_for_messages(
        &self,
        task_id: &str,
        message_ids: &[i32],
    ) -> Result<HashMap<i32, Vec<TaskMessageContext>>, sqlx::Error>;

    /// Look up a single message by its public uuid. Returns `None` when no row
    /// matches.
    async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TaskMessage>, sqlx::Error>;

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

    /// Write a run's usage onto the last top-level `assistant` row of
    /// `turn_number`. Returns `false` when that row already has usage or the
    /// turn has no such row; an earlier row of the turn is never written.
    #[expect(
        clippy::too_many_arguments,
        reason = "one column per usage tier, bound as-is"
    )]
    async fn backfill_last_assistant_usage<'a>(
        &self,
        task_id: &'a str,
        turn_number: i32,
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
    async fn get_last_message_type(
        &self,
        task_id: &str,
    ) -> Result<Option<super::models::MessageType>, sqlx::Error>;

    /// Whether `turn_number` has a final assistant reply: its latest shown
    /// Trajectory line is assistant text with no tool call — or, for a batched
    /// turn, a later turn already got one, since coalesced runs attribute it
    /// there.
    async fn last_message_is_completed_reply(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
    ) -> Result<bool, sqlx::Error>;
    async fn get_latest_message_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;

    async fn set_external_source_metadata<'a>(
        &self,
        message_uuid: &'a str,
        external_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<bool, sqlx::Error>;

    /// Record an external event this Task produced, for a caller with no
    /// `tool_use_id` to hang it on — a `moat` command in a sandbox shell. See
    /// [`outbound_events`](crate::task::outbound_events_dao).
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

    /// Like [`Self::has_external_event_id`] but matches any candidate id. Used
    /// by the dedup gate to recognize both the new `<provider>:<kind>:<id>`
    /// form and any legacy bare-int form in one round-trip.
    async fn has_any_external_event_id(
        &self,
        task_id: &str,
        external_ids: &[String],
    ) -> Result<bool, sqlx::Error>;

    async fn get_external_event_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error>;

    /// Find the owning task for any of the given external IDs. Matches the id
    /// alone across every subject, so it only suits ids a provider allocates
    /// once (e.g. a comment id).
    ///
    /// Use
    /// [`find_task_id_by_any_external_event_id_on_subject`](Self::find_task_id_by_any_external_event_id_on_subject)
    /// for ids a second subject can repeat.
    async fn find_task_id_by_any_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error>;

    /// Find the owning task among events carried on `subject`: `task_messages`
    /// is scoped by its subject key, `task_outbound_events` by its source name;
    /// a row carrying neither is not a match.
    ///
    /// Used by adapter delivery to attach a binding to the authoring task while
    /// sibling bindings on the subject still receive it.
    async fn find_task_id_by_any_external_event_id_on_subject<'a>(
        &self,
        external_ids: &[String],
        subject: EventSubject<'a>,
    ) -> Result<Option<String>, sqlx::Error>;

    /// Keyset-paginated page of messages strictly older than `before_id` (or
    /// the most recent `limit` rows when `before_id` is `None`). Returned rows
    /// are in chronological (oldest-first) order.
    async fn get_paginated(
        &self,
        task_id: &str,
        before_id: Option<i32>,
        limit: i32,
    ) -> Result<MessagePage, sqlx::Error>;

    /// Load exactly the rows to re-assemble the Message for `message_uuid`,
    /// plus the upsert's target `uuid` — the named uuid *is* the target; a
    /// carrier row is redirected on the way in, not here.
    ///
    /// Returns `None` for a subagent row (rides the progress path) or a uuid
    /// with neither text nor activity.
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
    /// the assistant's `tool_use` input plus the matching `tool_result` content
    /// (if a result has been recorded yet).
    async fn find_tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRow>, sqlx::Error>;

    /// Whether `file_id` is referenced by any message in `task_id` via the
    /// persisted `attachments.file_ids` JSON. Used by the sandbox
    /// `DownloadUpload` RPC to defense-in-depth the cross-task check.
    async fn file_id_used_in_task(&self, task_id: &str, file_id: &str)
    -> Result<bool, sqlx::Error>;

    /// Locate the most recent user-turn that was acknowledged by the sandbox
    /// (`agent_session_id` set), has this Task's own `task_turns` row, and has
    /// no terminal event (`completed/error/stopped`) on it or any later turn.
    /// Returns `None` when no such turn exists.
    async fn find_interrupted_turn(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, sqlx::Error>;

    /// Whether the turn produced agent output after its most recent
    /// `agent.recovery_started`: `true` marks a legitimate re-interruption,
    /// `false` a crash-loop that never resumed.
    ///
    /// Resets the attempt counter on `true`, so only a no-progress loop
    /// exhausts the cap.
    async fn turn_progressed_since_last_recovery(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<bool, sqlx::Error>;

    /// The parent's agent session at a fork cut, and the transcript line the
    /// cut lands on. `None` when the parent's agent never ran at or before it.
    async fn fork_anchors(
        &self,
        task_id: &str,
        at_turn: Option<i32>,
    ) -> Result<Option<crate::message::fork_dao::ForkAnchors>, sqlx::Error>;

    /// Copy a Task's conversation onto a fork, up to and including `at_turn`.
    /// See [`crate::message::fork_dao`] for what the copy carries and re-keys.
    async fn fork_conversation(
        &self,
        source_task_id: &str,
        dest_task_id: &str,
        at_turn: Option<i32>,
    ) -> Result<crate::message::models::ForkCopyCounts, sqlx::Error>;

    /// Delete a fork's copied conversation, undoing [`Self::fork_conversation`]
    /// when the create meant to follow it fails. See
    /// [`crate::message::fork_dao::discard_conversation`] for when it's safe.
    async fn discard_forked_conversation(&self, dest_task_id: &str) -> Result<(), sqlx::Error>;
}

/// PostgreSQL implementation of [`MessageDao`].
pub(crate) struct PgMessageDao(pub(crate) DbPool);

/// The columns a whole Message is written with, and their placeholders.
///
/// A macro, not a `const`, since sqlx 0.9 needs `&'static str` concatenated at
/// compile time; paired with [`bind_message`] to keep both lists in sync
/// without drifting silently.
macro_rules! message_columns {
    () => {
        "task_id, uuid, message_type, user_type, parent_uuid,
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
         tool_response, effort, external_author, skills,
         external_sender_id, sent_at"
    };
}
pub(crate) use message_columns;

macro_rules! message_placeholders {
    () => {
        "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
         $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
         $21, $22, $23, $24, $25, $26, $27, $28, $29, $30,
         $31, $32, $33, $34, $35, $36, $37, $38, $39, $40,
         $41, $42, $43, $44, $45, $46, $47"
    };
}
pub(crate) use message_placeholders;

/// Bind one `TaskMessage` to the placeholders [`message_columns`] declares.
///
/// A macro because the two callers hold different query types — `query_as` for
/// the upsert, `query` for the fork copy — and `bind` is inherent to each.
macro_rules! bind_message {
    ($q:expr, $msg:expr) => {
        $q.bind(&$msg.task_id)
            .bind(&$msg.uuid)
            .bind($msg.message_type.as_str())
            .bind(&$msg.user_type)
            .bind(&$msg.parent_uuid)
            .bind($msg.is_sidechain)
            .bind(&$msg.cwd)
            .bind(&$msg.session_id)
            .bind($msg.turn_number)
            .bind(&$msg.version)
            .bind(&$msg.git_branch)
            .bind(&$msg.request_id)
            .bind(&$msg.timestamp)
            .bind($msg.message_role.as_str())
            .bind(&$msg.message_text)
            .bind(&$msg.assistant_message_id)
            .bind(&$msg.assistant_message_type)
            .bind(&$msg.assistant_message_model)
            .bind(&$msg.assistant_stop_reason)
            .bind(&$msg.assistant_stop_sequence)
            .bind($msg.assistant_usage_input_tokens)
            .bind($msg.assistant_usage_output_tokens)
            .bind($msg.assistant_usage_cache_creation_input_tokens)
            .bind($msg.assistant_usage_cache_read_input_tokens)
            .bind(&$msg.tool_use_result)
            .bind(&$msg.sdk_metadata)
            .bind(&$msg.attachments)
            .bind(&$msg.agent_session_id)
            .bind($msg.will_not_resume)
            .bind(&$msg.run_metrics)
            .bind($msg.agent_mode_raw.as_deref())
            .bind(&$msg.model)
            .bind($msg.resume)
            .bind(&$msg.external_event_id)
            .bind(&$msg.external_adapter)
            .bind(&$msg.external_source_name)
            .bind(&$msg.source_task_id)
            .bind($msg.start_fresh)
            .bind(&$msg.sent_by)
            .bind(&$msg.external_subject_key)
            .bind(&$msg.assistant_usage)
            .bind(&$msg.tool_response)
            .bind(&$msg.effort)
            .bind(&$msg.external_author)
            // Deliberately absent from the upsert's `DO UPDATE SET`: that path
            // is the per-block agent-message merge, and must not blank a user
            // row's skills.
            .bind(&$msg.skills)
            .bind(&$msg.external_sender_id)
            // Absent from the upsert's `DO UPDATE SET`: the ingest path always
            // binds NULL here, and re-upserting a Message must not unsend one
            // `acknowledge_messages` has already stamped.
            .bind($msg.sent_at)
    };
}
pub(crate) use bind_message;

/// Stamps the Messages of the delivered items `ids` with their run, inside the
/// caller's transaction; their position does not move.
pub(crate) async fn stamp_delivered_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_id: &str,
    ids: &[uuid::Uuid],
    run: &crate::inbox::RunStamp,
) -> Result<(), sqlx::Error> {
    let uuids: Vec<String> = ids.iter().map(ToString::to_string).collect();
    sqlx::query(
        r#"UPDATE task_messages
              SET sent_at = NOW(), session_id = $3, agent_session_id = $3, turn_number = $4
            WHERE task_id = $1 AND uuid = ANY($2) AND agent_session_id IS NULL"#,
    )
    .bind(task_id)
    .bind(&uuids)
    .bind(&run.session_id)
    .bind(run.turn_number)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// [`MessageDao::append`] inside the caller's transaction.
pub(crate) async fn append_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    msg: &TaskMessage,
    contexts: &[TaskMessageContext],
    conversation: &ConversationWrite,
) -> Result<i32, sqlx::Error> {
    let row: (i32,) = bind_message!(
        sqlx::query_as(concat!(
            "INSERT INTO task_messages (",
            message_columns!(),
            ") VALUES (",
            message_placeholders!(),
            r#")
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
        RETURNING id"#
        )),
        msg
    )
    .fetch_one(&mut **tx)
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
            .execute(&mut **tx)
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
        .execute(&mut **tx)
        .await?;
    }

    // Text, spliced at each block's own index.
    //
    // `message_text_parts[i]` is subscripted assignment, which enlarges the
    // array — and creates it when the column is NULL — so an out-of-order or
    // redelivered text block lands in the same slot either way. That idempotency
    // is why an append would not do. PostgreSQL subscripts from 1.
    for part in &conversation.text {
        sqlx::query("UPDATE task_messages SET message_text_parts[$2] = $3 WHERE id = $1")
            .bind(message_id)
            .bind(i32::try_from(part.block_index).unwrap_or(i32::MAX) + 1)
            .bind(&part.text)
            .execute(&mut **tx)
            .await?;
    }
    if !conversation.text.is_empty() {
        // `array_to_string` skips NULL holes, which is what the slots for
        // blocks that are not text are. The separator is the blank line
        // markdown needs between paragraphs.
        sqlx::query(
            r#"UPDATE task_messages
                  SET message_text = array_to_string(message_text_parts, E'\n\n')
                WHERE id = $1"#,
        )
        .bind(message_id)
        .execute(&mut **tx)
        .await?;
    }

    // The conversation's position, allocated once for the Message and every
    // activity that happened on it, so they interleave in one stream.
    let position = match msg.parent_uuid.as_deref() {
        None => {
            moatless_core::change_feed::conversation::record_message_change(
                tx,
                &msg.task_id,
                &msg.uuid,
            )
            .await?
        }
        // A subagent's rows are never in the Task's own stream, so they
        // allocate nothing from its counter — see the note below on why they
        // record no change. They still need a position to sort by within
        // their own transcript.
        Some(parent_uuid) => next_subagent_position(tx, &msg.task_id, parent_uuid).await?,
    };

    for activity in &conversation.activities {
        task_activities::dao::upsert(tx, position, activity).await?;
    }
    for outcome in &conversation.outcomes {
        task_activities::dao::settle(tx, outcome).await?;
    }
    if let Some(usage) = conversation.usage {
        task_activities::dao::admit_usage(
            tx,
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
        task_dao::clear_settled_for_all_followers(tx, &msg.task_id).await?;
        moatless_core::change_feed::listing::record_change(tx, &msg.task_id).await?;
    }

    Ok(message_id)
}

/// [`MessageDao::append_activities`] inside the caller's transaction.
pub(crate) async fn append_activities_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_id: &str,
    message_uuid: &str,
    conversation: &ConversationWrite,
) -> Result<(), sqlx::Error> {
    if conversation.is_empty() {
        return Ok(());
    }

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
        Some(parent_uuid) => next_subagent_position(tx, task_id, parent_uuid).await?,
        None => {
            moatless_core::change_feed::conversation::record_message_change(
                tx,
                task_id,
                message_uuid,
            )
            .await?
        }
    };

    for activity in &conversation.activities {
        task_activities::dao::upsert(tx, position, activity).await?;
    }
    for outcome in &conversation.outcomes {
        task_activities::dao::settle(tx, outcome).await?;
    }
    if let Some(usage) = conversation.usage {
        task_activities::dao::admit_usage(tx, task_id, message_uuid, parent_uuid, usage).await?;
    }

    if conversation.activities.iter().any(|activity| {
        activity.kind == ActivityKind::ToolCall
            && INTERACTIVE_TOOLS.contains(&activity.summary.as_str())
    }) {
        moatless_core::change_feed::listing::record_change(tx, task_id).await?;
    }

    Ok(())
}

/// The agent mode of the Task's latest user Message, which an agent's own
/// rows inherit.
pub(crate) async fn last_user_agent_mode_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT agent_mode FROM task_messages
            WHERE task_id = $1 AND message_type = 'user'
            ORDER BY timestamp DESC
            LIMIT 1"#,
    )
    .bind(task_id)
    .fetch_optional(&mut **tx)
    .await
    .map(Option::flatten)
}

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
        let message_id = append_in(&mut tx, msg, contexts, conversation).await?;
        tx.commit().await?;
        Ok(message_id)
    }

    async fn append_error_result(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
        error_text: &str,
    ) -> Result<(), sqlx::Error> {
        let event = crate::task_event::models::InsertTaskEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            event_type: moatless_types::event::EventType::Task.as_str().to_string(),
            sub_type: moatless_types::task::TaskSubType::Error
                .as_str()
                .to_string(),
            is_error: true,
            agent_profile: None,
            turn_number,
            payload: serde_json::json!({ "text": error_text }),
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        let mut tx = self.0.begin().await?;
        crate::task_event::dao::insert_in(&mut tx, &event).await?;
        crate::conversation::projector::Projector
            .on_task_error(&mut tx, &event)
            .await?;
        tx.commit().await
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
        append_activities_in(&mut tx, task_id, message_uuid, conversation).await?;
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
        let position = moatless_core::change_feed::conversation::record_activity_change(
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
        turn_number: i32,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        cache_creation_input_tokens: Option<i32>,
        cache_read_input_tokens: Option<i32>,
        assistant_usage: Option<&'a serde_json::Value>,
    ) -> Result<bool, sqlx::Error> {
        // Pick the turn's last row first, then gate on it. Selecting the last
        // row *without* usage instead lets a redelivered completion write the
        // run's total a second time, onto an earlier row of the same turn.
        let result = sqlx::query(
            r#"UPDATE task_messages SET
                   assistant_usage_input_tokens = $3,
                   assistant_usage_output_tokens = $4,
                   assistant_usage_cache_creation_input_tokens = $5,
                   assistant_usage_cache_read_input_tokens = $6,
                   assistant_usage = $7
               WHERE id = (
                   SELECT id FROM task_messages
                   WHERE task_id = $1
                     AND turn_number = $2
                     AND message_type = 'assistant'
                     AND parent_uuid IS NULL
                   ORDER BY timestamp DESC, id DESC
                   LIMIT 1
               )
                 AND assistant_usage IS NULL"#,
        )
        .bind(task_id)
        .bind(turn_number)
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
        // A queued item's Message is the inbox's to stamp, when it is delivered.
        let result = sqlx::query(
            r#"UPDATE task_messages m
               SET session_id = $2, agent_session_id = $2, sent_at = NOW()
               WHERE m.task_id = $1
                 AND (m.message_type = 'user' OR m.message_type = 'error')
                 AND m.agent_session_id IS NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM task_inbox i
                      WHERE i.task_id = m.task_id AND i.id::text = m.uuid
                        AND i.status = 'pending'
                 )"#,
        )
        .bind(task_id)
        .bind(session_id)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected())
    }

    async fn get_last_message_type(
        &self,
        task_id: &str,
    ) -> Result<Option<super::models::MessageType>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"SELECT message_type FROM task_messages
               WHERE task_id = $1
               ORDER BY timestamp DESC, id DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;
        row.map(|r| decode_required("message_type", r.0))
            .transpose()
    }

    async fn last_message_is_completed_reply(
        &self,
        task_id: &str,
        turn_number: Option<i32>,
    ) -> Result<bool, sqlx::Error> {
        crate::message::recovery_dao::last_message_is_completed_reply(&self.0, task_id, turn_number)
            .await
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

    async fn set_external_source_metadata<'a>(
        &self,
        message_uuid: &'a str,
        external_id: &'a str,
        provider: &'a str,
        source_name: Option<&'a str>,
    ) -> Result<bool, sqlx::Error> {
        crate::message::external_events_dao::set_source_metadata(
            &self.0,
            message_uuid,
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
        crate::task::outbound_events_dao::record(
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
        crate::message::external_events_dao::has(&self.0, task_id, external_id).await
    }

    async fn has_any_external_event_id(
        &self,
        task_id: &str,
        external_ids: &[String],
    ) -> Result<bool, sqlx::Error> {
        crate::message::external_events_dao::has_any(&self.0, task_id, external_ids).await
    }

    async fn get_external_event_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
        crate::message::external_events_dao::ids_for_task(&self.0, task_id).await
    }

    async fn find_task_id_by_any_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error> {
        crate::message::external_events_dao::task_for_any(&self.0, external_ids).await
    }

    async fn find_task_id_by_any_external_event_id_on_subject<'a>(
        &self,
        external_ids: &[String],
        subject: EventSubject<'a>,
    ) -> Result<Option<String>, sqlx::Error> {
        crate::message::external_events_dao::task_for_any_on_subject(&self.0, external_ids, subject)
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
        crate::message::recovery_dao::find_interrupted_turn(&self.0, task_id).await
    }

    async fn turn_progressed_since_last_recovery(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<bool, sqlx::Error> {
        crate::message::recovery_dao::turn_progressed_since_last_recovery(
            &self.0,
            task_id,
            turn_number,
        )
        .await
    }

    async fn fork_anchors(
        &self,
        task_id: &str,
        at_turn: Option<i32>,
    ) -> Result<Option<crate::message::fork_dao::ForkAnchors>, sqlx::Error> {
        crate::message::fork_dao::anchors(&self.0, task_id, at_turn).await
    }

    async fn fork_conversation(
        &self,
        source_task_id: &str,
        dest_task_id: &str,
        at_turn: Option<i32>,
    ) -> Result<crate::message::models::ForkCopyCounts, sqlx::Error> {
        crate::message::fork_dao::copy_conversation(&self.0, source_task_id, dest_task_id, at_turn)
            .await
    }

    async fn discard_forked_conversation(&self, dest_task_id: &str) -> Result<(), sqlx::Error> {
        crate::message::fork_dao::discard_conversation(&self.0, dest_task_id).await
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_pending_work_db_tests.rs"]
mod dao_pending_work_db_tests;

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
