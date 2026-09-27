//! The Projector: turns inbox items and Trajectory lines into the Messages
//! and activities a conversation shows, in the transaction that wrote the
//! source.

use std::collections::HashMap;

use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::inbox::{InboxItem, InboxOrigin, RunStamp};
use crate::message::models::{MessageRole, MessageType, TaskMessage};
use crate::task_activities::mapping::{
    ActivityStatus, AdmittedUsage, BlockContext, ConversationWrite, ToolOutcome, map_content,
    user_input_resolved,
};
use crate::task_event::models::InsertTaskEvent;
use crate::trajectory::models::{LineProjection, NewEvent, TrajectorySession};

/// The only writer of `task_messages` and `task_activities` rows from an inbox
/// item or a Trajectory line.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Projector;

impl Projector {
    /// Writes the item's Message, `uuid` = the item's id, positioned now. A
    /// `sandbox` item is nobody's message and projects nothing.
    pub(crate) async fn on_inbox(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: &InboxItem,
    ) -> Result<(), sqlx::Error> {
        if matches!(item.origin, InboxOrigin::Sandbox) {
            return Ok(());
        }
        let message = item.as_message();
        crate::message::dao::append_in(tx, &message, &item.context_rows(0), &answer_of(item))
            .await?;
        Ok(())
    }

    /// Stamps the delivered items' Messages with their run; their position does
    /// not move.
    pub(crate) async fn on_delivered(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_id: &str,
        ids: &[uuid::Uuid],
        run: &RunStamp,
    ) -> Result<(), sqlx::Error> {
        crate::message::dao::stamp_delivered_in(tx, task_id, ids, run).await?;
        Ok(())
    }

    /// Projects a session's appended lines, `first_seq` onward, by the rules
    /// of the plan's section 7. Reads each line's `content`, never its
    /// `payload` or `kind`.
    pub(crate) async fn on_lines(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        session: &TrajectorySession,
        first_seq: i64,
        lines: &[(Uuid, &NewEvent)],
        turn_number: Option<i32>,
    ) -> Result<LineProjection, sqlx::Error> {
        let task_id = session.task_id.as_str();
        let mut projection = LineProjection::default();
        let parent_uuid = match session.parent_event_id {
            Some(event) => crate::trajectory::dao::spawning_call_in(tx, event).await?,
            None => None,
        };
        let mut agent_mode: Option<Option<String>> = None;
        // Each Message's next block index; a Claude response continues across
        // lines, and across appends.
        let mut next_block: HashMap<String, usize> = HashMap::new();
        for (offset, (event_id, line)) in lines.iter().enumerate() {
            let seq = first_seq + offset as i64;
            let content = &line.content;
            if session.own_from_seq.is_some_and(|own| seq < own) || content.is_null() {
                continue;
            }
            let flag = |key: &str| content.get(key).and_then(Value::as_bool) == Some(true);
            if flag("injected") {
                continue;
            }
            let uuid = line
                .harness_item_id
                .clone()
                .unwrap_or_else(|| event_id.to_string());
            let usage = content.get("usage").and_then(admitted_usage);
            let (message_type, message_role) = match content.get("role").and_then(Value::as_str) {
                Some("assistant") => (MessageType::Assistant, MessageRole::Assistant),
                Some("user") => (MessageType::System, MessageRole::User),
                _ => {
                    if let Some(usage) = usage {
                        crate::task_activities::dao::admit_usage(
                            tx,
                            task_id,
                            &uuid,
                            parent_uuid.as_deref(),
                            usage,
                        )
                        .await?;
                    }
                    continue;
                }
            };
            let blocks = content.get("blocks").cloned().unwrap_or(Value::Null);
            let block_index = match next_block.get(&uuid) {
                Some(index) => *index,
                // First in this batch: an earlier append may hold its first blocks.
                None if line.harness_item_id.is_some() => {
                    let held = crate::trajectory::dao::blocks_before_in(tx, session.id, &uuid, seq)
                        .await?;
                    usize::try_from(held).unwrap_or(0)
                }
                None => 0,
            };
            next_block.insert(
                uuid.clone(),
                block_index + blocks.as_array().map_or(0, Vec::len),
            );
            let mut write = map_content(
                BlockContext {
                    task_id,
                    message_uuid: &uuid,
                    parent_uuid: parent_uuid.as_deref(),
                    message_type,
                    block_index,
                },
                &blocks,
            );
            write.usage = usage;
            if message_role == MessageRole::User {
                // The inbox owns what was sent; the CLI's copy is never shown.
                write.text.clear();
            } else {
                projection.skills.extend(skills_in(&blocks));
            }

            if !write.text.is_empty() {
                if agent_mode.is_none() {
                    agent_mode =
                        Some(crate::message::dao::last_user_agent_mode_in(tx, task_id).await?);
                }
                let mode = agent_mode.clone().flatten();
                let message = agent_message(AgentLine {
                    task_id,
                    uuid: &uuid,
                    parent_uuid: parent_uuid.as_deref(),
                    session: &session.harness_session_id,
                    turn_number,
                    message_type,
                    message_role,
                    model: content.get("model").and_then(Value::as_str),
                    agent_mode: mode,
                });
                crate::message::dao::append_in(tx, &message, &[], &write).await?;
                self.settled(tx, task_id, &write, &mut projection).await?;
                touch(&mut projection, parent_uuid.as_deref(), &uuid);
                continue;
            }
            if write.is_empty() {
                continue;
            }
            // A result changes the Message that called for it, and a derived
            // line shows under the call it views; the carrier is not rendered.
            let anchor_call = content
                .get("derived_from")
                .and_then(Value::as_str)
                .or_else(|| {
                    write
                        .activities
                        .is_empty()
                        .then(|| write.outcomes.first().map(|o| o.tool_use_id.as_str()))
                        .flatten()
                });
            let anchor = match anchor_call {
                Some(call) => crate::task_activities::dao::call_in(tx, task_id, call)
                    .await?
                    .and_then(|(message, _)| message)
                    .filter(|message| !message.is_empty())
                    .unwrap_or_else(|| uuid.clone()),
                None => uuid.clone(),
            };
            if content.get("derived_from").is_some() {
                for activity in &mut write.activities {
                    activity.message_uuid = Some(anchor.clone());
                }
            }
            crate::message::dao::append_activities_in(tx, task_id, &anchor, &write).await?;
            self.settled(tx, task_id, &write, &mut projection).await?;
            touch(&mut projection, parent_uuid.as_deref(), &anchor);
        }
        Ok(projection)
    }

    /// A `task.error` event becomes an Error Message whose uuid is the event's
    /// id, so projecting it again writes nothing new.
    pub(crate) async fn on_task_error(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        event: &InsertTaskEvent,
    ) -> Result<(), sqlx::Error> {
        let text = event
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message = TaskMessage {
            message_type: MessageType::Error,
            message_role: MessageRole::Assistant,
            user_type: "system".to_string(),
            turn_number: event.turn_number,
            message_text: Some(text.to_string()),
            will_not_resume: Some(false),
            ..TaskMessage::new_user_message(
                event.task_id.clone(),
                event.event_id.clone(),
                String::new(),
                event.timestamp.clone(),
            )
        };
        crate::message::dao::append_in(tx, &message, &[], &ConversationWrite::default()).await?;
        Ok(())
    }

    /// Records the calls `write` settled, with their tool names.
    async fn settled(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_id: &str,
        write: &ConversationWrite,
        projection: &mut LineProjection,
    ) -> Result<(), sqlx::Error> {
        for outcome in &write.outcomes {
            let name = crate::task_activities::dao::call_in(tx, task_id, &outcome.tool_use_id)
                .await?
                .map(|(_, name)| name);
            projection.settled.push((name, outcome.is_error));
        }
        Ok(())
    }
}

/// The skills an assistant line's `Skill` calls load, for the Task's
/// `skill-<name>` tags.
fn skills_in(blocks: &Value) -> Vec<String> {
    blocks
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| {
            b.get("type").and_then(Value::as_str) == Some("tool_use")
                && b.get("name").and_then(Value::as_str) == Some("Skill")
        })
        .filter_map(|b| b.pointer("/input/skill").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn touch(projection: &mut LineProjection, parent_uuid: Option<&str>, uuid: &str) {
    match parent_uuid {
        Some(parent) => projection.touch_subagent(parent),
        None => projection.touch_message(uuid),
    }
}

/// Section 4.1's `usage`, as the conversation admits it.
fn admitted_usage(usage: &Value) -> Option<AdmittedUsage> {
    let tokens = |key: &str| {
        usage
            .get(key)
            .and_then(Value::as_i64)
            .map_or(0, |n| i32::try_from(n).unwrap_or(i32::MAX))
    };
    usage.is_object().then(|| AdmittedUsage {
        input_tokens: tokens("input_tokens"),
        output_tokens: tokens("output_tokens"),
        cache_creation_input_tokens: tokens("cache_write_tokens"),
        cache_read_input_tokens: tokens("cache_read_tokens"),
    })
}

struct AgentLine<'a> {
    task_id: &'a str,
    uuid: &'a str,
    parent_uuid: Option<&'a str>,
    session: &'a str,
    turn_number: Option<i32>,
    message_type: MessageType,
    message_role: MessageRole,
    model: Option<&'a str>,
    agent_mode: Option<String>,
}

/// The row an agent's line with text writes; its text is spliced in by block.
fn agent_message(line: AgentLine<'_>) -> TaskMessage {
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let assistant = line.message_type == MessageType::Assistant;
    TaskMessage {
        message_type: line.message_type,
        message_role: line.message_role,
        user_type: "system".to_string(),
        parent_uuid: line.parent_uuid.map(str::to_string),
        session_id: Some(line.session.to_string()),
        turn_number: line.turn_number,
        message_text: None,
        assistant_message_id: assistant.then(|| line.uuid.to_string()),
        assistant_message_model: assistant.then(|| line.model.map(str::to_string)).flatten(),
        agent_mode_raw: line.agent_mode,
        ..TaskMessage::new_user_message(
            line.task_id.to_string(),
            line.uuid.to_string(),
            String::new(),
            timestamp,
        )
    }
}

/// A tool response settles the call it answers: without the outcome the
/// question keeps rendering as unanswered, and the resolved row beside it is
/// what closes the panel.
fn answer_of(item: &InboxItem) -> ConversationWrite {
    let (Some(tool_use_id), Some(response)) = (
        item.answers_tool_use_id.as_deref(),
        item.options.tool_response.as_ref(),
    ) else {
        return ConversationWrite::default();
    };
    let answer: serde_json::Map<String, serde_json::Value> = response
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !matches!(key.as_str(), "toolUseId" | "toolName"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let answer = if answer.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Object(answer)
    };
    let uuid = item.id.to_string();
    ConversationWrite {
        outcomes: vec![ToolOutcome {
            task_id: item.task_id.clone(),
            tool_use_id: tool_use_id.to_string(),
            status: ActivityStatus::Completed,
            result: if answer.is_null() {
                serde_json::Value::String(item.text.clone())
            } else {
                answer.clone()
            },
            typed_result: None,
            is_error: false,
        }],
        activities: vec![user_input_resolved(
            &item.task_id,
            &uuid,
            tool_use_id,
            &answer,
        )],
        ..Default::default()
    }
}
