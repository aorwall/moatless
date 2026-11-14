//! The Projector: turns inbox items, Trajectory lines and task events into the
//! Messages and activities a conversation shows, in the transaction that wrote
//! the source.

use std::collections::HashMap;

use serde_json::Value;
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use super::dao::{self, MessageRow, Source};
use super::ids;
use crate::activities::mapping::{
    ActivityKind, ActivityRow, ActivityStatus, AdmittedUsage, BlockContext, ConversationWrite,
    ToolOutcome, agent_switched, compaction, context_window_updated, map_content, sandbox_activity,
    user_input_resolved,
};
use crate::activities::start_steps::is_start_step;
use crate::inbox::{InboxItem, InboxOrigin, RunStamp};
use crate::message::models::MessageType;
use crate::task_event::models::InsertTaskEvent;
use crate::trajectory::models::{LineProjection, NewEvent, TrajectorySession};
use moatless_types::activity::{ArtifactShown, SecretRequestAnswered, SecretRequested};
use moatless_types::event::EventType;
use moatless_types::sandbox::SandboxStatus;
use moatless_types::task::TaskSubType;

/// Keep in sync with `handlers::INTERACTIVE_TOOL_NAMES` and
/// `moatless_t3code::dao`'s pair: raising one moves a sidebar listing row.
const INTERACTIVE_TOOLS: &[&str] = &["AskUserQuestion", "ExitPlanMode"];

/// One Trajectory line as the Projector reads it: `content`, never `payload`
/// or `kind`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Line<'a> {
    pub id: Uuid,
    pub seq: i64,
    pub harness_item_id: Option<&'a str>,
    pub content: &'a Value,
    pub turn_number: Option<i32>,
}

/// The only writer of `conversation_*` rows.
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
        self.project_item(tx, &item.task_id, item, true).await
    }

    /// Sets the delivered items' turn on their Messages; their position does
    /// not move.
    pub(crate) async fn on_delivered(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_id: &str,
        ids: &[Uuid],
        run: &RunStamp,
    ) -> Result<(), sqlx::Error> {
        dao::stamp_delivered(tx, task_id, ids, run.turn_number).await
    }

    /// Projects a session's appended lines, `first_seq` onward, by the rules
    /// of the plan's section 7.
    pub(crate) async fn on_lines(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        session: &TrajectorySession,
        first_seq: i64,
        lines: &[(Uuid, &NewEvent)],
        turn_number: Option<i32>,
    ) -> Result<LineProjection, sqlx::Error> {
        let lines: Vec<Line<'_>> = lines
            .iter()
            .enumerate()
            .map(|(offset, (id, line))| Line {
                id: *id,
                seq: first_seq + offset as i64,
                harness_item_id: line.harness_item_id.as_deref(),
                content: &line.content,
                turn_number,
            })
            .collect();
        self.project_lines(tx, &session.task_id, session, &lines, true)
            .await
    }

    /// Projects a persisted task event, once: an error, a turn result, a
    /// Sandbox change, a context-window reading or a reply sent through a
    /// binding. Anything else, or an event already projected, projects
    /// nothing. Called by every writer of the row and by the dispatcher's
    /// redelivery, since a writer outside this crate cannot call it.
    pub(crate) async fn on_task_event(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        event: &InsertTaskEvent,
    ) -> Result<(), sqlx::Error> {
        if !projects(&event.event_type, &event.sub_type)
            || dao::event_projected(tx, &event.task_id, &event.event_id).await?
        {
            return Ok(());
        }
        self.project_event(tx, &event.task_id, &event.into()).await
    }

    /// Settles the interactive call `tool_use_id` with `answer`, which the
    /// Task's own inbox item `item` carried as a plain message.
    pub(crate) async fn on_answer(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_id: &str,
        item: Uuid,
        tool_use_id: &str,
        answer: &Value,
    ) -> Result<(), sqlx::Error> {
        let name = item.to_string();
        let mut resolved = user_input_resolved(task_id, &name, tool_use_id, answer);
        resolved.activity_id = format!("{}:user-input-resolved", ids::scope(task_id, &name));
        let position = dao::position_for(tx, task_id, None, item).await?;
        dao::upsert_activity(tx, position, &resolved, Source::Inbox(item)).await?;
        let outcome = ToolOutcome {
            task_id: task_id.to_string(),
            tool_use_id: tool_use_id.to_string(),
            status: ActivityStatus::Completed,
            result: answer.clone(),
            typed_result: None,
            is_error: false,
        };
        dao::settle(tx, &outcome, None).await?;
        Ok(())
    }

    pub(super) async fn project_item(
        &self,
        tx: &mut PgConnection,
        task_id: &str,
        item: &InboxItem,
        live: bool,
    ) -> Result<(), sqlx::Error> {
        if matches!(item.origin, InboxOrigin::Sandbox) {
            return Ok(());
        }
        let own = item.task_id == task_id;
        let uuid = if own {
            item.id
        } else {
            ids::message(task_id, &item.id.to_string())
        };
        let position = dao::upsert_message(
            tx,
            &MessageRow {
                uuid,
                task_id,
                author: "user",
                kind: "text",
                text: None,
                parent_tool_use_id: None,
                turn_number: item.turn_number,
                harness: None,
                model: None,
                usage: None,
                source: Source::Inbox(item.id),
            },
        )
        .await?;
        if let Some((outcome, mut resolved)) = answer_of(item) {
            resolved.task_id = task_id.to_string();
            resolved.activity_id = format!(
                "{}:user-input-resolved",
                ids::scope(task_id, &item.id.to_string())
            );
            resolved.message_uuid = Some(uuid.to_string());
            dao::upsert_activity(tx, position, &resolved, Source::Inbox(item.id)).await?;
            dao::settle(
                tx,
                &ToolOutcome {
                    task_id: task_id.to_string(),
                    ..outcome
                },
                None,
            )
            .await?;
        }
        if live {
            crate::task::dao::clear_settled_for_all_followers(tx, task_id).await?;
            moatless_core::change_feed::listing::record_change(tx, task_id).await?;
        }
        Ok(())
    }

    pub(super) async fn project_lines(
        &self,
        tx: &mut PgConnection,
        task_id: &str,
        session: &TrajectorySession,
        lines: &[Line<'_>],
        live: bool,
    ) -> Result<LineProjection, sqlx::Error> {
        let mut projection = LineProjection::default();
        let parent = match session.parent_event_id {
            Some(event) => crate::trajectory::dao::spawning_call_in(tx, event).await?,
            None => None,
        };
        let parent = parent.as_deref();
        // Each Message's next block index; a Claude response continues across
        // lines, and across appends.
        let mut next_block: HashMap<Uuid, usize> = HashMap::new();
        let mut interactive = false;
        for line in lines {
            let content = line.content;
            if session.own_from_seq.is_some_and(|own| line.seq < own) || content.is_null() {
                continue;
            }
            if content.get("injected").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            // Claude's stand-in for a reply no model wrote: its usage is no
            // spend, and only an API error ("API Error: 529 …") is shown, not
            // the "No response requested." of a resumed unanswered prompt.
            let synthetic = content.get("model").and_then(Value::as_str) == Some("<synthetic>");
            if synthetic && content.get("api_error").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            // A line that stands in for an earlier reply (Codex's compaction
            // summary, which it also sent as a reply) removes that reply.
            if let Some(item) = content.get("supersedes").and_then(Value::as_str) {
                if let Some(first) = dao::first_line_of_item(tx, session.id, item).await? {
                    dao::delete_message(tx, task_id, ids::message(task_id, &first.to_string()))
                        .await?;
                }
            }
            if content.get("compaction").and_then(Value::as_bool) == Some(true) {
                // A subagent's compaction is its own; only the Task's shows.
                if parent.is_none() {
                    let name = format!("{}:compaction", ids::scope(task_id, &line.id.to_string()));
                    let row = compaction(&name, task_id, &compaction_summary(content));
                    dao::standalone_activity(tx, &row, Source::Line(line.id)).await?;
                }
                continue;
            }
            let first = match line.harness_item_id {
                Some(item) => dao::first_line_of_item(tx, session.id, item)
                    .await?
                    .unwrap_or(line.id),
                None => line.id,
            };
            let uuid = ids::message(task_id, &first.to_string());
            let usage = content
                .get("usage")
                .filter(|_| !synthetic)
                .and_then(admitted_usage);
            if let Some(usage) = usage {
                dao::admit_usage(tx, task_id, line.id, line.harness_item_id, usage).await?;
            }
            let (message_type, assistant) = match content.get("role").and_then(Value::as_str) {
                Some("assistant") => (MessageType::Assistant, true),
                Some("user") => (MessageType::System, false),
                _ => continue,
            };
            let blocks = content.get("blocks").cloned().unwrap_or(Value::Null);
            let block_index = match next_block.get(&uuid) {
                Some(index) => *index,
                // First in this batch: an earlier append may hold its first blocks.
                None => match line.harness_item_id {
                    Some(item) => {
                        let held = crate::trajectory::dao::blocks_before_in(
                            tx, session.id, item, line.seq,
                        )
                        .await?;
                        usize::try_from(held).unwrap_or(0)
                    }
                    None => 0,
                },
            };
            next_block.insert(uuid, block_index + blocks.as_array().map_or(0, Vec::len));
            let message_uuid = uuid.to_string();
            let source = ids::scope(task_id, &line.id.to_string());
            let mut write = map_content(
                BlockContext {
                    task_id,
                    message_uuid: &message_uuid,
                    parent_uuid: parent,
                    message_type,
                    block_index,
                    source: &source,
                    source_block: 0,
                },
                &blocks,
            );
            if assistant {
                projection.skills.extend(skills_in(&blocks));
            } else {
                // The inbox owns what was sent; the CLI's copy is never shown.
                write.text.clear();
            }
            interactive |= write.activities.iter().any(|a| {
                a.kind == ActivityKind::ToolCall && INTERACTIVE_TOOLS.contains(&a.summary.as_str())
            });

            if !write.text.is_empty() {
                let position = dao::upsert_message(
                    tx,
                    &MessageRow {
                        uuid,
                        task_id,
                        author: "agent",
                        kind: "text",
                        text: None,
                        parent_tool_use_id: parent,
                        turn_number: line.turn_number,
                        harness: Some(&session.harness),
                        model: content.get("model").and_then(Value::as_str),
                        usage: None,
                        source: Source::Line(first),
                    },
                )
                .await?;
                dao::splice_text(tx, uuid, &write.text).await?;
                self.write_activities(tx, position, &write, line.id, &mut projection)
                    .await?;
                touch(&mut projection, parent, &message_uuid);
                continue;
            }
            if write.activities.is_empty() && write.outcomes.is_empty() {
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
                Some(call) => dao::call_in(tx, task_id, call)
                    .await?
                    .and_then(|(message, _)| message)
                    .filter(|message| !message.is_empty())
                    .unwrap_or_else(|| message_uuid.clone()),
                None => message_uuid.clone(),
            };
            if content.get("derived_from").is_some() {
                for activity in &mut write.activities {
                    activity.message_uuid = Some(anchor.clone());
                }
            }
            let anchor_id = Uuid::parse_str(&anchor).unwrap_or(uuid);
            let position = dao::position_for(tx, task_id, parent, anchor_id).await?;
            self.write_activities(tx, position, &write, line.id, &mut projection)
                .await?;
            touch(&mut projection, parent, &anchor);
        }
        if live && interactive {
            crate::task::dao::clear_settled_for_all_followers(tx, task_id).await?;
            moatless_core::change_feed::listing::record_change(tx, task_id).await?;
        }
        Ok(projection)
    }

    /// Writes a line's activities at `position` and settles the calls it
    /// answers, recording their tool names.
    async fn write_activities(
        &self,
        tx: &mut PgConnection,
        position: i64,
        write: &ConversationWrite,
        line: Uuid,
        projection: &mut LineProjection,
    ) -> Result<(), sqlx::Error> {
        for activity in &write.activities {
            dao::upsert_activity(tx, position, activity, Source::Line(line)).await?;
        }
        for outcome in &write.outcomes {
            dao::settle(tx, outcome, Some(line)).await?;
            let name = dao::call_in(tx, &outcome.task_id, &outcome.tool_use_id)
                .await?
                .map(|(_, name)| name);
            projection.settled.push((name, outcome.is_error));
        }
        Ok(())
    }

    pub(super) async fn project_event(
        &self,
        tx: &mut PgConnection,
        task_id: &str,
        event: &EventSource<'_>,
    ) -> Result<(), sqlx::Error> {
        let Ok(event_type) = EventType::try_from(event.event_type) else {
            return Ok(());
        };
        let uuid = ids::message(task_id, event.event_id);
        let name = ids::scope(task_id, event.event_id);
        match event_type {
            EventType::Task => match TaskSubType::try_from(event.sub_type) {
                Ok(TaskSubType::Error) => {
                    let text = event.payload.get("text").and_then(Value::as_str);
                    self.system_message(tx, task_id, uuid, "error", text, event, None)
                        .await?;
                }
                Ok(TaskSubType::AgentSwitched) => {
                    let field = |key: &str| event.payload.get(key).and_then(Value::as_str);
                    let summary = field("summary")
                        .filter(|summary| !summary.is_empty())
                        .unwrap_or("Switched agent");
                    let chosen = event.payload.get("chosen").and_then(Value::as_bool) == Some(true);
                    let row =
                        agent_switched(&name, task_id, summary, field("from"), field("to"), chosen);
                    dao::standalone_activity(tx, &row, Source::TaskEvent(event.event_id)).await?;
                }
                Ok(TaskSubType::ArtifactShown) => {
                    let Ok(shown) = serde_json::from_value::<ArtifactShown>(event.payload.clone())
                    else {
                        return Ok(());
                    };
                    let row = artifact_shown(&name, task_id, &shown);
                    dao::standalone_activity(tx, &row, Source::TaskEvent(event.event_id)).await?;
                }
                Ok(TaskSubType::SecretRequested) => {
                    let Ok(asked) =
                        serde_json::from_value::<SecretRequested>(event.payload.clone())
                    else {
                        return Ok(());
                    };
                    let row = secret_requested(&name, task_id, &asked);
                    dao::standalone_activity(tx, &row, Source::TaskEvent(event.event_id)).await?;
                }
                Ok(TaskSubType::SecretRequestAnswered) => {
                    let Ok(answered) =
                        serde_json::from_value::<SecretRequestAnswered>(event.payload.clone())
                    else {
                        return Ok(());
                    };
                    let row = secret_request_answered(&name, task_id, &answered);
                    dao::standalone_activity(tx, &row, Source::TaskEvent(event.event_id)).await?;
                }
                Ok(TaskSubType::BindingReplySent) => {
                    let Some(reply) = super::payload::BindingReply::of(event.payload) else {
                        return Ok(());
                    };
                    if !reply.shown {
                        return Ok(());
                    }
                    dao::upsert_message(
                        tx,
                        &MessageRow {
                            uuid,
                            task_id,
                            author: "agent",
                            kind: "text",
                            text: Some(&reply.text),
                            parent_tool_use_id: None,
                            turn_number: event.turn_number,
                            harness: None,
                            model: None,
                            usage: None,
                            source: Source::TaskEvent(event.event_id),
                        },
                    )
                    .await?;
                }
                _ => {}
            },
            EventType::Agent => match event.sub_type {
                "completed"
                    if !event.is_error && !super::payload::is_error_result(event.payload) =>
                {
                    let usage = match event.turn_number {
                        Some(turn) => dao::turn_usage(tx, task_id, turn).await?,
                        None => None,
                    };
                    self.system_message(tx, task_id, uuid, "turn_result", None, event, usage)
                        .await?;
                }
                "context_window" => {
                    let Some(used) = event
                        .payload
                        .get("usedTokens")
                        .and_then(Value::as_i64)
                        .filter(|used| *used >= 0)
                    else {
                        return Ok(());
                    };
                    let max = event
                        .payload
                        .get("maxTokens")
                        .and_then(Value::as_i64)
                        .filter(|max| *max > 0);
                    let row = context_window_updated(&name, task_id, used, max);
                    dao::standalone_activity(tx, &row, Source::TaskEvent(event.event_id)).await?;
                }
                "recording_stopped" => {
                    let message = match event.payload.get("parentHarnessSessionId") {
                        Some(_) => "Subagent recording stopped: its later messages are not shown",
                        None => "Conversation recording stopped: later messages are not shown",
                    };
                    let payload = moatless_types::sandbox::SandboxEventPayload {
                        message: message.to_string(),
                        ..Default::default()
                    };
                    let mut row = sandbox_activity(&name, task_id, "error", true, &payload);
                    if let Some(error) = event.payload.get("error").and_then(Value::as_str) {
                        row.payload["detail"] = Value::String(error.to_string());
                    }
                    dao::standalone_activity(tx, &row, Source::TaskEvent(event.event_id)).await?;
                }
                _ => {}
            },
            EventType::Sandbox => {
                if is_start_step(event.sub_type, event.payload) {
                    return Ok(());
                }
                let (coarse, is_error) = match SandboxStatus::try_from(event.sub_type) {
                    Ok(SandboxStatus::Error) => ("error", true),
                    Ok(
                        status @ (SandboxStatus::Initializing
                        | SandboxStatus::Ready
                        | SandboxStatus::Stopped),
                    ) => (status.as_str(), false),
                    _ => return Ok(()),
                };
                let payload: moatless_types::sandbox::SandboxEventPayload =
                    serde_json::from_value(event.payload.clone()).unwrap_or_default();
                let row = sandbox_activity(&name, task_id, coarse, is_error, &payload);
                if !dao::restates_sandbox(tx, &row).await? {
                    dao::standalone_activity(tx, &row, Source::TaskEvent(event.event_id)).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one Message, its columns as they are"
    )]
    async fn system_message(
        &self,
        tx: &mut PgConnection,
        task_id: &str,
        uuid: Uuid,
        kind: &'static str,
        text: Option<&str>,
        event: &EventSource<'_>,
        usage: Option<Value>,
    ) -> Result<(), sqlx::Error> {
        dao::upsert_message(
            tx,
            &MessageRow {
                uuid,
                task_id,
                author: "system",
                kind,
                text,
                parent_tool_use_id: None,
                turn_number: event.turn_number,
                harness: None,
                model: None,
                usage,
                source: Source::TaskEvent(event.event_id),
            },
        )
        .await?;
        Ok(())
    }
}

/// A task event as the Projector reads it, from an insert or a stored row.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EventSource<'a> {
    pub event_id: &'a str,
    pub event_type: &'a str,
    pub sub_type: &'a str,
    pub is_error: bool,
    pub turn_number: Option<i32>,
    pub payload: &'a Value,
}

impl<'a> From<&'a InsertTaskEvent> for EventSource<'a> {
    fn from(event: &'a InsertTaskEvent) -> Self {
        Self {
            event_id: &event.event_id,
            event_type: &event.event_type,
            sub_type: &event.sub_type,
            is_error: event.is_error,
            turn_number: event.turn_number,
            payload: &event.payload,
        }
    }
}

/// An Artifact version shown inline in the thread, which happened on no
/// Message.
fn artifact_shown(event_id: &str, task_id: &str, shown: &ArtifactShown) -> ActivityRow {
    ActivityRow {
        activity_id: event_id.to_string(),
        task_id: task_id.to_string(),
        ordinal: 0,
        message_uuid: None,
        parent_uuid: None,
        kind: ActivityKind::ArtifactShown,
        status: ActivityStatus::Completed,
        tool_use_id: None,
        summary: shown.title.clone(),
        payload: serde_json::to_value(shown).unwrap_or(Value::Null),
        is_error: false,
    }
}

/// A secret an agent asked a person for, which happened on no Message.
fn secret_requested(event_id: &str, task_id: &str, asked: &SecretRequested) -> ActivityRow {
    ActivityRow {
        activity_id: event_id.to_string(),
        task_id: task_id.to_string(),
        ordinal: 0,
        message_uuid: None,
        parent_uuid: None,
        kind: ActivityKind::SecretRequested,
        status: ActivityStatus::Completed,
        tool_use_id: None,
        summary: asked.label.clone(),
        payload: serde_json::to_value(asked).unwrap_or(Value::Null),
        is_error: false,
    }
}

fn secret_request_answered(
    event_id: &str,
    task_id: &str,
    answered: &SecretRequestAnswered,
) -> ActivityRow {
    ActivityRow {
        activity_id: event_id.to_string(),
        task_id: task_id.to_string(),
        ordinal: 0,
        message_uuid: None,
        parent_uuid: None,
        kind: ActivityKind::SecretRequestAnswered,
        status: ActivityStatus::Completed,
        tool_use_id: None,
        summary: format!("Secret request {}", answered.outcome),
        payload: serde_json::to_value(answered).unwrap_or(Value::Null),
        is_error: false,
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

fn touch(projection: &mut LineProjection, parent: Option<&str>, uuid: &str) {
    match parent {
        Some(parent) => projection.touch_subagent(parent),
        None => projection.touch_message(uuid),
    }
}

/// The text a compaction line kept, its text blocks joined.
fn compaction_summary(content: &Value) -> String {
    content
        .get("blocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n")
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

/// A tool response settles the call it answers: without the outcome the
/// question keeps rendering as unanswered, and the resolved row beside it is
/// what closes the panel.
fn answer_of(item: &InboxItem) -> Option<(ToolOutcome, ActivityRow)> {
    let tool_use_id = item.answers_tool_use_id.as_deref()?;
    let response = item.options.tool_response.as_ref()?;
    let answer: serde_json::Map<String, Value> = response
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !matches!(key.as_str(), "toolUseId" | "toolName"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let answer = if answer.is_empty() {
        Value::Null
    } else {
        Value::Object(answer)
    };
    let outcome = ToolOutcome {
        task_id: item.task_id.clone(),
        tool_use_id: tool_use_id.to_string(),
        status: ActivityStatus::Completed,
        result: if answer.is_null() {
            Value::String(item.text.clone())
        } else {
            answer.clone()
        },
        typed_result: None,
        is_error: false,
    };
    let resolved = user_input_resolved(&item.task_id, &item.id.to_string(), tool_use_id, &answer);
    Some((outcome, resolved))
}

/// Whether [`Projector::project_event`] reads events of this type.
fn projects(event_type: &str, sub_type: &str) -> bool {
    matches!(
        (event_type, sub_type),
        (
            "task",
            "error"
                | "binding_reply_sent"
                | "agent_switched"
                | "artifact_shown"
                | "secret_requested"
                | "secret_request_answered"
        ) | (
            "agent",
            "completed" | "context_window" | "recording_stopped"
        ) | ("sandbox", "initializing" | "ready" | "stopped" | "error")
    )
}
