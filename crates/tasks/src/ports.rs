//! Tasks' side of the seams declared in `moatless_backend_core::task`.
//!
//! Each adapter wraps one store and narrows it to the projection a peer crate
//! reads or the entry it writes, converting `sqlx::Error` into a `StoreError`
//! that boxes it. The composition root builds these once and hands out the port
//! a consumer needs.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_backend_core::store::StoreError;
use moatless_backend_core::task::{
    AssistantReply, ExternalEvent, TagResolver, TaskDescriptor, TaskReader, ToolCallRecord,
    TranscriptAppend, TranscriptEntry, TranscriptEntryKind, TranscriptOrigin, TurnCounter,
};
use moatless_types::access_control::ScopeContext;

use crate::message::dao::MessageDao;
use crate::message::models::{MessageRole, MessageType, TaskMessage};
use crate::tag::dao::TagDao;
use crate::task::dao::TaskDao;
use crate::task_activities::mapping::ConversationWrite;
use crate::task_turn::dao::TaskTurnDao;

/// The `tasks` table, narrowed to the four facts a peer reads.
pub struct TaskProjection {
    tasks: Arc<dyn TaskDao>,
}

impl TaskProjection {
    pub fn new(tasks: Arc<dyn TaskDao>) -> Self {
        Self { tasks }
    }
}

#[async_trait]
impl TaskReader for TaskProjection {
    async fn describe(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskDescriptor>, StoreError> {
        let task = self
            .tasks
            .find_by_id(task_id, ctx)
            .await
            .map_err(|e| StoreError::with_source(format!("read task {task_id}"), e))?;
        Ok(task.map(|task| TaskDescriptor {
            task_id: task.task_id,
            status: task.status,
            name: task.name,
            owner_user_id: task.owner_user_id,
            workspace_id: task.workspace_id,
            created_by: task.created_by,
        }))
    }

    async fn may_access(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, StoreError> {
        Ok(self.describe(task_id, ctx).await?.is_some())
    }
}

/// The `task_messages` table, narrowed to the transcript operations a peer
/// performs.
pub struct Transcript {
    messages: Arc<dyn MessageDao>,
}

impl Transcript {
    pub fn new(messages: Arc<dyn MessageDao>) -> Self {
        Self { messages }
    }
}

#[async_trait]
impl TranscriptAppend for Transcript {
    async fn append(&self, entry: &TranscriptEntry) -> Result<(), StoreError> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut msg = TaskMessage::new_user_message(
            entry.task_id.clone(),
            entry.uuid.clone(),
            entry.text.clone(),
            now,
        );
        msg.message_type = match entry.kind {
            TranscriptEntryKind::AssistantReply => MessageType::Assistant,
            TranscriptEntryKind::OutboundStub => MessageType::System,
        };
        msg.message_role = MessageRole::Assistant;
        msg.user_type = "assistant".to_string();
        msg.turn_number = entry.turn_number;
        msg.external_adapter = entry.external_adapter.clone();
        msg.external_source_name = entry.external_source_name.clone();

        self.messages
            .append(&msg, &[], &ConversationWrite::default())
            .await
            .map_err(|e| {
                StoreError::with_source(format!("append to task {} transcript", entry.task_id), e)
            })?;
        Ok(())
    }

    async fn set_external_source_metadata(
        &self,
        message_uuid: &str,
        event: &ExternalEvent,
    ) -> Result<bool, StoreError> {
        self.messages
            .set_external_source_metadata(
                message_uuid,
                &event.external_id,
                &event.provider,
                event.source_name.as_deref(),
            )
            .await
            .map_err(|e| StoreError::with_source(format!("tag message {message_uuid}"), e))
    }

    async fn record_external_event(
        &self,
        task_id: &str,
        event: &ExternalEvent,
    ) -> Result<(), StoreError> {
        self.messages
            .record_external_event(
                task_id,
                &event.external_id,
                &event.provider,
                event.source_name.as_deref(),
            )
            .await
            .map_err(|e| {
                StoreError::with_source(format!("record external event for task {task_id}"), e)
            })
    }

    async fn find_by_external_event_id(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, StoreError> {
        self.messages
            .find_task_id_by_any_external_event_id(external_ids)
            .await
            .map_err(|e| StoreError::with_source("find the task that authored an event", e))
    }

    async fn external_event_ids(&self, task_id: &str) -> Result<Vec<String>, StoreError> {
        self.messages
            .get_external_event_ids(task_id)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read external event ids for task {task_id}"), e)
            })
    }

    async fn find_by_uuid(&self, uuid: &str) -> Result<Option<TranscriptOrigin>, StoreError> {
        let msg = self
            .messages
            .find_by_uuid(uuid)
            .await
            .map_err(|e| StoreError::with_source(format!("read message {uuid}"), e))?;
        Ok(msg.map(|msg| TranscriptOrigin {
            task_id: msg.task_id,
            external_event_id: msg.external_event_id,
            external_adapter: msg.external_adapter,
            external_subject_key: msg.external_subject_key,
        }))
    }

    async fn last_assistant_reply(
        &self,
        task_id: &str,
    ) -> Result<Option<AssistantReply>, StoreError> {
        let msg = self
            .messages
            .get_last_assistant_message(task_id)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read the last reply on task {task_id}"), e)
            })?;
        Ok(msg.map(|msg| AssistantReply {
            uuid: msg.uuid,
            text: msg.message_text,
        }))
    }

    async fn tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<ToolCallRecord>, StoreError> {
        let row = self
            .messages
            .find_tool_call(task_id, tool_use_id)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read tool call {tool_use_id} on {task_id}"), e)
            })?;
        Ok(row.map(|row| ToolCallRecord {
            tool_name: row.tool_name,
            tool_input: row.tool_input,
            result_content: row.result_content,
            is_error: row.is_error,
        }))
    }
}

/// The `task_turns` table, narrowed to the Turn a row appended now belongs to.
pub struct TurnLedger {
    turns: Arc<dyn TaskTurnDao>,
}

impl TurnLedger {
    pub fn new(turns: Arc<dyn TaskTurnDao>) -> Self {
        Self { turns }
    }
}

#[async_trait]
impl TurnCounter for TurnLedger {
    async fn latest_turn_number(&self, task_id: &str) -> Result<Option<i32>, StoreError> {
        self.turns
            .latest_turn_number(task_id)
            .await
            .map_err(|e| StoreError::with_source(format!("read the turn number of {task_id}"), e))
    }
}

/// The `tags` table, narrowed to name-to-id resolution.
pub struct TagCatalog {
    tags: Arc<dyn TagDao>,
}

impl TagCatalog {
    pub fn new(tags: Arc<dyn TagDao>) -> Self {
        Self { tags }
    }
}

#[async_trait]
impl TagResolver for TagCatalog {
    async fn resolve(&self, names: &[String]) -> Result<Vec<String>, StoreError> {
        let tags = self
            .tags
            .find_or_create_tags(names)
            .await
            .map_err(|e| StoreError::with_source("resolve tag names", e))?;
        Ok(tags.into_iter().map(|tag| tag.tag_id).collect())
    }
}
