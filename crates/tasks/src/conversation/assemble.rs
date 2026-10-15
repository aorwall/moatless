//! Conversation rows as the `TaskMessage`s every reader serves: a user
//! Message's fields from its inbox item, a line's session from the Trajectory,
//! a system Message's run from its task event, and a frozen row from `legacy`.

use std::collections::HashMap;

use moatless_core::db::DbPool;
use moatless_types::agent::AgentMode;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::payload;
use super::read_dao::{self, MessageRecord};
use crate::inbox::{InboxItem, InboxOrigin};
use crate::message::models::{MessageRole, MessageType, TaskMessage, TaskMessageContext};

/// `records` as Messages with their contexts, keyed by the Message `id`,
/// which is its position. An agent Message takes the agent mode of the user
/// Message before it; `mode` is the one in force before the first record.
pub(crate) async fn load(
    pool: &DbPool,
    records: &[MessageRecord],
    mode: Option<AgentMode>,
) -> Result<(Vec<TaskMessage>, Vec<TaskMessageContext>), sqlx::Error> {
    let ids: Vec<Uuid> = records.iter().filter_map(|r| r.inbox_id).collect();
    let items = read_dao::inbox_items(pool, &ids).await?;
    Ok(assemble(records, &items, mode))
}

/// One record as a Message, reading the agent mode it ran under.
pub(crate) async fn load_one(
    pool: &DbPool,
    record: MessageRecord,
) -> Result<TaskMessage, sqlx::Error> {
    let mode = if record.author == "agent" {
        read_dao::agent_mode_at(pool, &record.task_id, record.position).await?
    } else {
        None
    };
    let (mut messages, _) = load(pool, std::slice::from_ref(&record), mode).await?;
    Ok(messages.remove(0))
}

pub(crate) fn assemble(
    records: &[MessageRecord],
    items: &HashMap<Uuid, InboxItem>,
    mut mode: Option<AgentMode>,
) -> (Vec<TaskMessage>, Vec<TaskMessageContext>) {
    let mut messages = Vec::with_capacity(records.len());
    let mut contexts = Vec::new();
    for record in records {
        let id = i32::try_from(record.position).unwrap_or(i32::MAX);
        let item = record.inbox_id.and_then(|inbox| items.get(&inbox));
        if let Some(item) = item {
            contexts.extend(item.context_rows(id));
        }
        let mut message = match (&record.legacy, item) {
            (Some(Value::Object(legacy)), _) if record.frozen => frozen(record, legacy),
            (_, Some(item)) => user(record, item),
            _ => projected(record, mode.clone()),
        };
        message.id = id;
        // A frozen user row has an inbox item too; 0178 gave it the origin.
        message.sandbox_wake = item.is_some_and(|item| matches!(item.origin, InboxOrigin::Sandbox));
        if message.message_type == MessageType::User {
            mode.clone_from(&message.agent_mode);
        }
        messages.push(message);
    }
    (messages, contexts)
}

fn timestamp(record: &MessageRecord) -> String {
    record
        .created_at
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn user(record: &MessageRecord, item: &InboxItem) -> TaskMessage {
    let base = item.as_message();
    TaskMessage {
        task_id: record.task_id.clone(),
        uuid: record.uuid.to_string(),
        turn_number: record.turn_number.or(item.turn_number),
        session_id: item.session_id.clone(),
        agent_session_id: item.session_id.clone(),
        sent_at: item.delivered_at,
        // Inherited: the item is its ancestor's.
        source_task_id: if item.task_id == record.task_id {
            base.source_task_id.clone()
        } else {
            Some(item.task_id.clone())
        },
        ..base
    }
}

fn projected(record: &MessageRecord, mode: Option<AgentMode>) -> TaskMessage {
    let (message_type, message_role) = match (record.author.as_str(), record.kind.as_str()) {
        ("user", _) => (MessageType::User, MessageRole::User),
        ("agent", _) => (MessageType::Assistant, MessageRole::Assistant),
        (_, "turn_result") => (MessageType::Result, MessageRole::Assistant),
        _ => (MessageType::Error, MessageRole::Assistant),
    };
    let agent = message_type == MessageType::Assistant;
    let meta = match (&record.event_sub_type, &record.event_payload) {
        (Some(sub_type), Some(payload)) if !agent => payload::result_meta(sub_type, payload),
        _ => payload::ResultMeta::default(),
    };
    let tokens = |key: &str| {
        record
            .usage
            .as_ref()
            .and_then(|usage| usage.get(key))
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
    };
    TaskMessage {
        message_type,
        message_role,
        user_type: "system".to_string(),
        sandbox_wake: false,
        parent_uuid: record.parent_tool_use_id.clone(),
        session_id: record.harness_session_id.clone(),
        turn_number: record.turn_number,
        assistant_message_id: agent.then(|| record.harness_item_id.clone()).flatten(),
        assistant_message_model: record.model.clone(),
        model: record.model.clone(),
        assistant_usage_input_tokens: tokens("input_tokens"),
        assistant_usage_output_tokens: tokens("output_tokens"),
        assistant_usage_cache_creation_input_tokens: tokens("cache_creation_input_tokens"),
        assistant_usage_cache_read_input_tokens: tokens("cache_read_input_tokens"),
        will_not_resume: meta.will_not_resume,
        run_metrics: meta.run_metrics,
        agent_mode: agent.then_some(mode).flatten(),
        created_at: Some(record.created_at),
        resume: false,
        ..TaskMessage::new_user_message(
            record.task_id.clone(),
            record.uuid.to_string(),
            String::new(),
            timestamp(record),
        )
    }
    .with_text(record.text.clone())
}

/// A frozen row: the columns it was frozen with, with the id, uuid, position
/// and text the conversation now holds.
fn frozen(record: &MessageRecord, legacy: &Map<String, Value>) -> TaskMessage {
    let text = |key: &str| legacy.get(key).and_then(Value::as_str).map(str::to_string);
    let int = |key: &str| {
        legacy
            .get(key)
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
    };
    let flag = |key: &str| legacy.get(key).and_then(Value::as_bool);
    let json = |key: &str| legacy.get(key).filter(|v| !v.is_null()).cloned();
    let time = |key: &str| {
        text(key)
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
            .map(|at| at.with_timezone(&chrono::Utc))
    };
    let message_type = text("message_type")
        .and_then(|t| MessageType::try_from(t).ok())
        .unwrap_or(MessageType::System);
    let message_role = text("message_role")
        .and_then(|r| MessageRole::try_from(r).ok())
        .unwrap_or(MessageRole::Assistant);
    TaskMessage {
        message_type,
        user_type: text("user_type").unwrap_or_else(|| "system".to_string()),
        sandbox_wake: false,
        parent_uuid: text("parent_uuid"),
        session_id: text("session_id"),
        turn_number: record.turn_number,
        message_role,
        assistant_message_id: text("assistant_message_id"),
        assistant_message_model: text("assistant_message_model"),
        assistant_stop_reason: text("assistant_stop_reason"),
        assistant_stop_sequence: text("assistant_stop_sequence"),
        assistant_usage_input_tokens: int("assistant_usage_input_tokens"),
        assistant_usage_output_tokens: int("assistant_usage_output_tokens"),
        assistant_usage_cache_creation_input_tokens: int(
            "assistant_usage_cache_creation_input_tokens",
        ),
        assistant_usage_cache_read_input_tokens: int("assistant_usage_cache_read_input_tokens"),
        tool_use_result: json("tool_use_result"),
        tool_response: json("tool_response"),
        sdk_metadata: json("sdk_metadata"),
        attachments: json("attachments"),
        agent_session_id: text("agent_session_id"),
        will_not_resume: flag("will_not_resume"),
        run_metrics: json("run_metrics"),
        agent_mode: text("agent_mode").map(AgentMode::from),
        model: text("model"),
        effort: text("effort"),
        skills: legacy
            .get("skills")
            .and_then(Value::as_array)
            .map(|skills| {
                skills
                    .iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        resume: flag("resume").unwrap_or(false),
        sent_at: time("sent_at"),
        created_at: Some(record.created_at),
        external_event_id: text("external_event_id"),
        external_adapter: text("external_adapter"),
        external_subject_key: text("external_subject_key"),
        external_source_name: text("external_source_name"),
        external_author: text("external_author"),
        external_sender_id: text("external_sender_id"),
        source_task_id: text("source_task_id"),
        start_fresh: flag("start_fresh").unwrap_or(false),
        sent_by: text("sent_by"),
        ..TaskMessage::new_user_message(
            record.task_id.clone(),
            record.uuid.to_string(),
            String::new(),
            text("timestamp").unwrap_or_else(|| timestamp(record)),
        )
    }
    .with_text(record.text.clone())
}

impl TaskMessage {
    fn with_text(self, message_text: Option<String>) -> Self {
        Self {
            message_text,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(position: i64, author: &str, legacy: Option<Value>) -> MessageRecord {
        MessageRecord {
            uuid: Uuid::new_v4(),
            task_id: "task-1".to_string(),
            position,
            author: author.to_string(),
            kind: "message".to_string(),
            text: Some("text".to_string()),
            parent_tool_use_id: None,
            turn_number: Some(1),
            model: None,
            usage: None,
            inbox_id: None,
            frozen: legacy.is_some(),
            legacy,
            created_at: chrono::Utc::now(),
            harness_session_id: None,
            harness_item_id: None,
            event_sub_type: None,
            event_payload: None,
        }
    }

    #[test]
    fn an_agent_message_runs_under_the_mode_of_the_user_message_before_it() {
        let user = serde_json::json!({
            "message_type": "user",
            "message_role": "user",
            "agent_mode": "plan",
        });
        let records = [
            record(1, "agent", None),
            record(2, "user", Some(user)),
            record(3, "agent", None),
        ];

        let (messages, _) = assemble(
            &records,
            &HashMap::new(),
            Some(AgentMode::from("code".to_string())),
        );

        let modes: Vec<_> = messages
            .iter()
            .map(|m| m.agent_mode.as_ref().map(AgentMode::as_str))
            .collect();
        assert_eq!(modes, vec![Some("code"), Some("plan"), Some("plan")]);
    }
}
