// This module compiles only for test builds (`#[cfg(test)]`) or downstream test
// tiers (`feature = "test-support"`), never a shipping build, so its callers are
// test scaffolding — allow them the way the crate's own `#[cfg(test)]` code is
// allowed (the crate-level allow is `test`-gated and does not cover the
// feature-gated build).
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_arguments)]

//! Named `db-tests` fixtures over the `task_messages` and `task_activities`
//! tables.
//!
//! A test that needs a transcript row, an activity or a page reaches the store
//! through one of these rather than through the store itself.

use std::sync::Arc;

use moatless_core::db::DbPool;
use moatless_ports::task::EventSubject;

use super::dao::{MessageDao, PgMessageDao};
use super::models::{MessagePage, MessageRole, MessageType, TaskMessage, TaskMessageContext};
use crate::task_activities::mapping::{ActivityRow, ConversationWrite};

fn messages(pool: &DbPool) -> PgMessageDao {
    PgMessageDao(pool.clone())
}

/// Append a Message with its contexts and content blocks, returning the
/// `task_message_id` the row was written with.
pub async fn seed_message(
    pool: &DbPool,
    msg: &TaskMessage,
    contexts: &[TaskMessageContext],
    conversation: &ConversationWrite,
) -> Result<i32, sqlx::Error> {
    messages(pool).append(msg, contexts, conversation).await
}

/// Every Message, context and activity a Task holds.
pub async fn messages_for(pool: &DbPool, task_id: &str) -> Result<MessagePage, sqlx::Error> {
    messages(pool).get_all(task_id).await
}

/// One keyset page of a Task's Messages, oldest first.
pub async fn paginated_messages(
    pool: &DbPool,
    task_id: &str,
    before_id: Option<i32>,
    limit: i32,
) -> Result<MessagePage, sqlx::Error> {
    messages(pool)
        .get_paginated(task_id, before_id, limit)
        .await
}

/// One subagent's Messages, contexts and activities.
pub async fn subagent_messages(
    pool: &DbPool,
    task_id: &str,
    tool_use_id: &str,
) -> Result<MessagePage, sqlx::Error> {
    messages(pool)
        .get_subagent_messages(task_id, tool_use_id)
        .await
}

/// The user Messages no turn has claimed yet.
pub async fn pending_user_messages(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<TaskMessage>, sqlx::Error> {
    messages(pool).get_pending_user(task_id).await
}

/// The most recent `result` Message.
pub async fn last_result_message(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskMessage>, sqlx::Error> {
    messages(pool).get_last_result(task_id).await
}

/// The most recent assistant Message.
pub async fn last_assistant_message(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskMessage>, sqlx::Error> {
    messages(pool).get_last_assistant_message(task_id).await
}

/// Stamp `session_id` on every Message of a Task that carries none.
pub async fn set_session_id(
    pool: &DbPool,
    task_id: &str,
    session_id: &str,
) -> Result<u64, sqlx::Error> {
    messages(pool)
        .set_session_id_for_task(task_id, session_id)
        .await
}

/// Write a run's usage onto the last assistant Message of a turn.
pub async fn backfill_last_assistant_usage(
    pool: &DbPool,
    task_id: &str,
    turn_number: i32,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    cache_creation_input_tokens: Option<i32>,
    cache_read_input_tokens: Option<i32>,
    assistant_usage: Option<&serde_json::Value>,
) -> Result<bool, sqlx::Error> {
    messages(pool)
        .backfill_last_assistant_usage(
            task_id,
            turn_number,
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
            assistant_usage,
        )
        .await
}

/// Mark Messages as claimed by a turn.
pub async fn acknowledge_messages(
    pool: &DbPool,
    task_id: &str,
    uuids: &[String],
    session_id: &str,
    turn_number: i32,
) -> Result<u64, sqlx::Error> {
    messages(pool)
        .acknowledge_messages(task_id, uuids, session_id, turn_number)
        .await
}

/// Write what a turn did when the turn wrote no Message of its own.
pub async fn seed_activities(
    pool: &DbPool,
    task_id: &str,
    message_uuid: &str,
    conversation: &ConversationWrite,
) -> Result<(), sqlx::Error> {
    messages(pool)
        .append_activities(task_id, message_uuid, conversation)
        .await
}

/// Write an activity that hangs off no Message. Returns whether a row was
/// written.
pub async fn record_standalone_activity(
    pool: &DbPool,
    activity: &ActivityRow,
) -> Result<bool, sqlx::Error> {
    messages(pool).record_standalone_activity(activity).await
}

/// Write a Sandbox status activity. Returns whether a row was written: a status
/// that restates the one before it writes none.
pub async fn record_sandbox_activity(
    pool: &DbPool,
    activity: &ActivityRow,
) -> Result<bool, sqlx::Error> {
    messages(pool).record_sandbox_activity(activity).await
}

/// Record an external event this Task produced.
pub async fn record_external_event(
    pool: &DbPool,
    task_id: &str,
    external_event_id: &str,
    provider: &str,
    source_name: Option<&str>,
) -> Result<(), sqlx::Error> {
    messages(pool)
        .record_external_event(task_id, external_event_id, provider, source_name)
        .await
}

/// Stamp the external source a Message came from.
pub async fn set_external_source_metadata(
    pool: &DbPool,
    message_uuid: &str,
    external_id: &str,
    provider: &str,
    source_name: Option<&str>,
) -> Result<bool, sqlx::Error> {
    messages(pool)
        .set_external_source_metadata(message_uuid, external_id, provider, source_name)
        .await
}

/// The Task that authored any one of `external_ids` on `subject`.
pub async fn task_id_for_external_event_on_subject(
    pool: &DbPool,
    external_ids: &[String],
    subject: EventSubject<'_>,
) -> Result<Option<String>, sqlx::Error> {
    messages(pool)
        .find_task_id_by_any_external_event_id_on_subject(external_ids, subject)
        .await
}

/// The published append seam over this pool, for a test that drives a writer
/// which takes one.
pub fn transcript(pool: &DbPool) -> Arc<crate::ports::Transcript> {
    Arc::new(crate::ports::Transcript::new(Arc::new(messages(pool))))
}

/// The Message handle over a pool that never connects, for a host test whose
/// path reaches the store only where a read failure is tolerated.
pub fn messages_without_a_database() -> Arc<super::operations::Messages> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .expect("a lazy pool connects to nothing");
    Arc::new(super::operations::Messages::new(Arc::new(messages(&pool))))
}

/// A minimal user Message, one line of text with no tool traffic.
pub fn make_message_row(task_id: &str, uuid: &str) -> TaskMessage {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    TaskMessage {
        id: 0, // ignored on insert (serial)
        task_id: task_id.to_string(),
        uuid: uuid.to_string(),
        message_type: MessageType::User,
        user_type: "human".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        turn_number: None,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: now,
        message_role: MessageRole::User,
        message_text: Some("hello".to_string()),
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result: None,
        tool_response: None,
        sdk_metadata: None,
        attachments: None,
        agent_session_id: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode_raw: None,
        model: None,
        effort: None,
        resume: true,
        skills: Vec::new(),
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_subject_key: None,
        external_source_name: None,
        external_author: None,
        sent_by: None,
        external_sender_id: None,
        source_task_id: None,
        start_fresh: false,
    }
}
