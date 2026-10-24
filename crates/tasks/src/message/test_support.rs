// Compiles only for test builds or `feature = "test-support"`, never a
// shipping build, so its callers are test scaffolding — allowed like the
// crate's own `#[cfg(test)]` code (the crate-level allow is `test`-gated and
// misses the feature-gated build).
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_arguments)]

//! Named `db-tests` fixtures over the conversation read model and the inbox.
//!
//! A test that needs a transcript row, an activity or a page reaches the store
//! through one of these rather than through the store itself. A seeded Message
//! is a frozen row, as history copied from before the read model is.

use std::sync::Arc;

use chrono::Utc;
use moatless_core::db::DbPool;
use moatless_ports::task::EventSubject;
use serde_json::{Value, json};
use uuid::Uuid;

use super::dao::{MessageDao, PgMessageDao};
use super::models::{MessagePage, MessageRole, MessageType, TaskMessage, TaskMessageContext};
use crate::activities::mapping::{ActivityRow, ConversationWrite};
use crate::conversation::dao as conversation;
use crate::inbox::{
    AdapterChannel, DeliveryOptions, ExternalSender, InboxContext, InboxOrigin, NewInboxItem,
};

fn messages(pool: &DbPool) -> PgMessageDao {
    PgMessageDao(pool.clone())
}

/// The conversation uuid a Message seeded as `id` takes: `id` itself when it
/// is a uuid.
pub fn seeded_uuid(id: &str) -> Uuid {
    id.parse()
        .unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("seed:{id}").as_bytes()))
}

/// The `activity_id` a row seeded or recorded as `name` is stored under.
pub fn seeded_activity_id(name: &str) -> String {
    crate::conversation::ids::activity(name).to_string()
}

/// Seed `msg` as a frozen Message with its contexts and conversation, and a
/// user one's delivered inbox item. Returns its position, the Message `id`.
pub async fn seed_message(
    pool: &DbPool,
    msg: &TaskMessage,
    contexts: &[TaskMessageContext],
    conversation: &ConversationWrite,
) -> Result<i32, sqlx::Error> {
    let uuid = seeded_uuid(&msg.uuid);
    let created_at = msg.created_at.unwrap_or_else(Utc::now);
    let text = (!conversation.text.is_empty())
        .then(|| {
            conversation
                .text
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .or_else(|| msg.message_text.clone());
    let mut tx = pool.begin().await?;
    let inbox_id = if msg.message_type == MessageType::User {
        let item = inbox_item(msg, uuid, text.clone().unwrap_or_default(), contexts);
        crate::inbox::dao::submit(&mut tx, &item).await?;
        crate::inbox::dao::seed_delivered(
            &mut tx,
            uuid,
            msg.sent_at.unwrap_or(created_at),
            msg.agent_session_id
                .as_deref()
                .or(msg.session_id.as_deref()),
            msg.turn_number,
        )
        .await?;
        crate::inbox::dao::clear_settled_pending_work(&mut tx, &msg.task_id).await?;
        Some(uuid)
    } else {
        None
    };
    let (author, kind) = match msg.message_type {
        MessageType::User => ("user", "text"),
        MessageType::Assistant => ("agent", "text"),
        MessageType::Error => ("system", "error"),
        MessageType::Result => ("system", "turn_result"),
        _ => ("system", "text"),
    };
    let position = conversation::seed_frozen_message(
        &mut tx,
        &conversation::FrozenSeed {
            uuid,
            task_id: &msg.task_id,
            author,
            kind,
            text: text.as_deref(),
            parent_tool_use_id: msg.parent_uuid.as_deref(),
            turn_number: msg.turn_number,
            model: msg.assistant_message_model.as_deref(),
            usage: None,
            inbox_id,
            legacy: legacy(msg),
            created_at,
        },
    )
    .await?;
    write_conversation(
        &mut tx,
        &msg.task_id,
        position,
        uuid,
        msg.parent_uuid.as_deref(),
        conversation,
    )
    .await?;
    tx.commit().await?;
    Ok(i32::try_from(position).unwrap_or(i32::MAX))
}

async fn write_conversation(
    tx: &mut sqlx::PgConnection,
    task_id: &str,
    position: i64,
    message: Uuid,
    parent: Option<&str>,
    conversation: &ConversationWrite,
) -> Result<(), sqlx::Error> {
    for row in &conversation.activities {
        let on = row.message_uuid.as_deref().map_or(message, seeded_uuid);
        conversation::seed_frozen_activity(&mut *tx, position, row, Some(on)).await?;
    }
    for outcome in &conversation.outcomes {
        conversation::settle_seeded(&mut *tx, outcome).await?;
    }
    if let Some(usage) = conversation.usage {
        conversation::seed_frozen_usage(&mut *tx, task_id, message, parent, usage).await?;
    }
    Ok(())
}

/// The inbox item a seeded user Message reads its origin and options from,
/// its origin derived as migration `0178` derives it.
fn inbox_item(
    msg: &TaskMessage,
    id: Uuid,
    text: String,
    contexts: &[TaskMessageContext],
) -> NewInboxItem {
    let origin = if msg.sandbox_wake {
        InboxOrigin::Sandbox
    } else if let Some(source_task_id) = &msg.source_task_id {
        InboxOrigin::Task {
            source_task_id: source_task_id.clone(),
            sent_by: msg.sent_by.clone(),
        }
    } else if let (Some(adapter), Some(event_id)) = (&msg.external_adapter, &msg.external_event_id)
    {
        InboxOrigin::Adapter {
            channel: AdapterChannel {
                adapter: adapter.clone(),
                external_message_id: event_id.clone(),
                subject_key: msg.external_subject_key.clone(),
                source_name: msg.external_source_name.clone(),
            },
            sender: ExternalSender {
                id: msg.external_sender_id.clone(),
                author: msg.external_author.clone(),
            },
            sent_by: msg.sent_by.clone(),
        }
    } else if let Some(sent_by) = &msg.sent_by {
        InboxOrigin::User {
            sent_by: sent_by.clone(),
        }
    } else {
        InboxOrigin::System
    };
    let file_ids = msg
        .attachments
        .as_ref()
        .and_then(|a| a.get("file_ids"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    NewInboxItem {
        id,
        task_id: msg.task_id.clone(),
        text,
        file_ids,
        options: DeliveryOptions {
            model: msg.model.clone(),
            agent_mode: msg
                .agent_mode
                .as_ref()
                .map(|mode| mode.as_str().to_string()),
            effort: msg.effort.clone(),
            skills: msg.skills.clone(),
            resume: msg.resume,
            start_fresh: msg.start_fresh,
            contexts: contexts
                .iter()
                .map(|c| InboxContext {
                    context_type: c.context_type.clone(),
                    reference_id: c.reference_id.clone(),
                    data: c.data.clone(),
                })
                .collect(),
            tool_response: msg.tool_response.clone(),
            agent_fallback: false,
            queue: false,
            child_outcome: None,
            record_notice: None,
        },
        answers_tool_use_id: msg
            .tool_response
            .as_ref()
            .and_then(|r| r.get("toolUseId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        origin,
    }
}

/// `msg` as a frozen `conversation_messages` row's `legacy` object, keyed by
/// the columns `0178_conversation_read_model.sql` copied.
fn legacy(msg: &TaskMessage) -> Value {
    let columns: [(&str, Value); 37] = [
        ("uuid", json!(msg.uuid)),
        ("message_type", json!(msg.message_type.as_str())),
        ("message_role", json!(msg.message_role.as_str())),
        ("user_type", json!(msg.user_type)),
        ("parent_uuid", json!(msg.parent_uuid)),
        ("session_id", json!(msg.session_id)),
        ("timestamp", json!(msg.timestamp)),
        ("assistant_message_id", json!(msg.assistant_message_id)),
        (
            "assistant_message_model",
            json!(msg.assistant_message_model),
        ),
        ("assistant_stop_reason", json!(msg.assistant_stop_reason)),
        (
            "assistant_stop_sequence",
            json!(msg.assistant_stop_sequence),
        ),
        (
            "assistant_usage_input_tokens",
            json!(msg.assistant_usage_input_tokens),
        ),
        (
            "assistant_usage_output_tokens",
            json!(msg.assistant_usage_output_tokens),
        ),
        (
            "assistant_usage_cache_creation_input_tokens",
            json!(msg.assistant_usage_cache_creation_input_tokens),
        ),
        (
            "assistant_usage_cache_read_input_tokens",
            json!(msg.assistant_usage_cache_read_input_tokens),
        ),
        ("tool_use_result", json!(msg.tool_use_result)),
        ("tool_response", json!(msg.tool_response)),
        ("sdk_metadata", json!(msg.sdk_metadata)),
        ("attachments", json!(msg.attachments)),
        ("agent_session_id", json!(msg.agent_session_id)),
        ("will_not_resume", json!(msg.will_not_resume)),
        ("run_metrics", json!(msg.run_metrics)),
        ("agent_mode", json!(msg.agent_mode)),
        ("model", json!(msg.model)),
        ("effort", json!(msg.effort)),
        ("skills", json!(msg.skills)),
        ("resume", json!(msg.resume)),
        ("sent_at", json!(msg.sent_at.map(|at| at.to_rfc3339()))),
        ("external_event_id", json!(msg.external_event_id)),
        ("external_adapter", json!(msg.external_adapter)),
        ("external_subject_key", json!(msg.external_subject_key)),
        ("external_source_name", json!(msg.external_source_name)),
        ("external_author", json!(msg.external_author)),
        ("external_sender_id", json!(msg.external_sender_id)),
        ("source_task_id", json!(msg.source_task_id)),
        ("start_fresh", json!(msg.start_fresh)),
        ("sent_by", json!(msg.sent_by)),
    ];
    Value::Object(
        columns
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

/// Every Message, context and activity a Task holds.
pub async fn messages_for(pool: &DbPool, task_id: &str) -> Result<MessagePage, sqlx::Error> {
    messages(pool).get_all(task_id).await
}

fn facade(pool: &DbPool) -> super::operations::Messages {
    super::operations::Messages::new(
        Arc::new(messages(pool)),
        crate::inbox::InboxService::new(pool.clone()),
    )
}

/// Submit an inbox item and project its Message, as a send path does.
/// `None` when an item with its id exists.
pub async fn queue_message(
    pool: &DbPool,
    item: crate::inbox::NewInboxItem,
) -> Result<Option<crate::inbox::InboxItem>, sqlx::Error> {
    super::send::queue(&crate::inbox::InboxService::new(pool.clone()), item).await
}

/// A person's message to `task_id`, `text` and nothing else.
pub fn user_item(task_id: &str, id: uuid::Uuid, text: &str) -> crate::inbox::NewInboxItem {
    crate::inbox::NewInboxItem {
        id,
        task_id: task_id.to_string(),
        text: text.to_string(),
        file_ids: Vec::new(),
        options: crate::inbox::DeliveryOptions::default(),
        answers_tool_use_id: None,
        origin: crate::inbox::InboxOrigin::User {
            sent_by: "user-1".to_string(),
        },
    }
}

/// Queue a person's item whose id derives from `name`; returns the id.
pub async fn queue_named(pool: &DbPool, task_id: &str, name: &str, text: &str) -> String {
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes());
    queue_message(pool, user_item(task_id, id, text))
        .await
        .unwrap()
        .expect("a new item");
    id.to_string()
}

/// The pending items no turn has taken yet, as the Messages the enricher reads.
pub async fn pending_user_messages(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<TaskMessage>, sqlx::Error> {
    Ok(facade(pool).pending_user(task_id).await?.messages)
}

/// The pending user Messages a fetch takes, less `queue` ones when
/// `hold_queued`.
pub async fn deliverable_user_messages(
    pool: &DbPool,
    task_id: &str,
    hold_queued: bool,
) -> Result<Vec<TaskMessage>, sqlx::Error> {
    Ok(facade(pool)
        .pending_deliverable(task_id, hold_queued)
        .await?
        .messages)
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

/// Mark inbox items delivered to a turn.
pub async fn acknowledge_messages(
    pool: &DbPool,
    task_id: &str,
    uuids: &[String],
    session_id: &str,
    turn_number: i32,
) -> Result<u64, sqlx::Error> {
    facade(pool)
        .acknowledge_messages(
            task_id,
            uuids,
            &crate::inbox::RunStamp {
                run_id: None,
                session_id: session_id.to_string(),
                turn_number,
            },
        )
        .await
}

/// Write what a turn did onto the seeded Message `message_uuid`, or in a change
/// of its own when no such Message is seeded.
pub async fn seed_activities(
    pool: &DbPool,
    task_id: &str,
    message_uuid: &str,
    conversation: &ConversationWrite,
) -> Result<(), sqlx::Error> {
    let message = seeded_uuid(message_uuid);
    let mut tx = pool.begin().await?;
    let position = match conversation::message_position(&mut tx, message).await? {
        Some(position) => position,
        None => conversation::position_for(&mut tx, task_id, None, message).await?,
    };
    write_conversation(&mut tx, task_id, position, message, None, conversation).await?;
    tx.commit().await
}

/// Mark `task_id` frozen, as the migration marks a Task created before
/// recording. A frozen call settles only when `open_since` is at or before
/// its write.
pub async fn freeze(
    pool: &DbPool,
    task_id: &str,
    open_since: Option<chrono::DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    conversation::seed_frozen_task(&mut tx, task_id, open_since).await?;
    tx.commit().await
}

/// Write an activity that hangs off no Message, as a task event projects one.
/// Returns whether a row was written.
pub async fn record_standalone_activity(
    pool: &DbPool,
    activity: &ActivityRow,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let written = conversation::standalone_activity(
        &mut tx,
        activity,
        conversation::Source::TaskEvent(&activity.activity_id),
    )
    .await?;
    tx.commit().await?;
    Ok(written)
}

/// Write a Sandbox status activity. Returns whether a row was written: a status
/// that restates the one before it writes none.
pub async fn record_sandbox_activity(
    pool: &DbPool,
    activity: &ActivityRow,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    if conversation::restates_sandbox(&mut tx, activity).await? {
        return Ok(false);
    }
    let written = conversation::standalone_activity(
        &mut tx,
        activity,
        conversation::Source::TaskEvent(&activity.activity_id),
    )
    .await?;
    tx.commit().await?;
    Ok(written)
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

/// A minimal user Message, one line of text with no tool traffic.
pub fn make_message_row(task_id: &str, uuid: &str) -> TaskMessage {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    TaskMessage {
        id: 0, // ignored on insert (serial)
        task_id: task_id.to_string(),
        uuid: uuid.to_string(),
        message_type: MessageType::User,
        user_type: "human".to_string(),
        sandbox_wake: false,
        parent_uuid: None,
        session_id: None,
        turn_number: None,
        timestamp: now,
        message_role: MessageRole::User,
        message_text: Some("hello".to_string()),
        assistant_message_id: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        tool_use_result: None,
        tool_response: None,
        sdk_metadata: None,
        attachments: None,
        agent_session_id: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode: None,
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

/// Append one agent text line to `task_id`'s Claude session `session`, as a
/// pod's write-out does, and return its event id. The line is projected.
pub async fn seed_agent_line(
    pool: &DbPool,
    task_id: &str,
    session: &str,
    turn_number: Option<i32>,
    text: &str,
) -> Uuid {
    seed_agent_blocks(
        pool,
        task_id,
        session,
        turn_number,
        json!([{ "type": "text", "text": text }]),
    )
    .await
}

/// Appends one Claude assistant line of `blocks`, projected as a live one is,
/// and returns its event id.
pub async fn seed_agent_blocks(
    pool: &DbPool,
    task_id: &str,
    session: &str,
    turn_number: Option<i32>,
    blocks: serde_json::Value,
) -> Uuid {
    use crate::trajectory::models::{AppendRequest, NewEvent};
    let store = crate::trajectory::dao::PgTrajectoryDao(pool.clone());
    let first_seq = store
        .find_session(session)
        .await
        .unwrap()
        .map_or(0, |s| s.last_seq + 1);
    let line = NewEvent {
        kind: "assistant".into(),
        harness_item_id: Some(format!("{session}-item-{first_seq}")),
        harness_turn_id: None,
        payload: json!({ "type": "assistant", "seq": first_seq }).to_string(),
        content: json!({ "role": "assistant", "blocks": blocks }),
    };
    store
        .append(AppendRequest {
            task_id,
            harness: moatless_types::agent::Harness::ClaudeCode,
            harness_session_id: session,
            first_seq,
            lines: std::slice::from_ref(&line),
            turn_number,
            subagent_parent: None,
            own_from_seq: None,
        })
        .await
        .unwrap();
    store
        .find_session(session)
        .await
        .unwrap()
        .and_then(|s| s.last_event_id)
        .expect("the appended line")
}
