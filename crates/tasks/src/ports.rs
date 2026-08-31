//! Tasks' side of the seams declared in `moatless_backend_core::task`.
//!
//! Each adapter wraps one store and narrows it to the projection a peer crate
//! reads or the entry it writes, converting `sqlx::Error` into a `StoreError`
//! that boxes it. The composition root builds these once and hands out the port
//! a consumer needs.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_backend_core::store::StoreError;
use moatless_backend_core::activity::{Activity, ActivitySource};
use moatless_backend_core::task::{
    AssistantReply, CloseReason, EventSubject, ExternalEvent, InboundTurn, MessageAcceptor, NewTask,
    RelationRecorder, SandboxProvision, TagResolver, TaskDescriptor, TaskFinisher, TaskOpenError,
    TaskOpened, TaskOpener, TaskReader, ToolCallRecord, TranscriptAppend, TranscriptEntry,
    TranscriptEntryKind, TranscriptOrigin, TurnCounter,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::task::RelatedTask;

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

    async fn find_author_on_subject<'a>(
        &self,
        external_ids: &[String],
        subject: EventSubject<'a>,
    ) -> Result<Option<String>, StoreError> {
        self.messages
            .find_task_id_by_any_external_event_id_on_subject(external_ids, subject)
            .await
            .map_err(|e| {
                StoreError::with_source("find the task that authored an event on a subject", e)
            })
    }

    async fn find_comment_author(
        &self,
        external_ids: &[String],
    ) -> Result<Option<String>, StoreError> {
        self.messages
            .find_task_id_by_any_external_event_id(external_ids)
            .await
            .map_err(|e| StoreError::with_source("find the task that authored a comment", e))
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

/// The provider path into task creation.
///
/// [`TaskCreationService::create`] is generic over two closures — the backend's
/// HTTP path enriches the success event and delivers a first message through
/// the whole `AppState` — so it is not dyn-safe. A Loop needs neither: it
/// carries event metadata and delivers its own first message. This port is that
/// entry, and the inherent method stays for the HTTP path.
#[async_trait]
impl TaskOpener for crate::task::creation::TaskCreationService {
    async fn open(&self, req: NewTask) -> Result<TaskOpened, TaskOpenError> {
        let metadata = req.event_metadata.clone();
        let outcome = self
            .create(
                crate::task::creation::NewTaskRequest {
                    repository_id: req.repository_id,
                    agent_type: req.agent_type,
                    name: req.name,
                    description: req.description,
                    branch: None,
                    model: None,
                    created_by: req.created_by,
                    headless: true,
                    visibility: req.visibility,
                    loop_id: Some(req.loop_id),
                    workspace_id: None,
                    // A Loop firing has no sandbox to join: it is triggered by
                    // an inbound event, not by a Task that already has one warm.
                    sandbox_id: None,
                    checkout_mode: None,
                    template_id: None,
                    vibe: None,
                    append_system_prompt: req.append_system_prompt,
                    ttl_minutes: req.ttl_minutes,
                    inactive_ttl_days: req.inactive_ttl_days,
                    tag_ids: req.tag_ids,
                    skills: req.skills,
                    parent: None,
                    related_to: req.related_to,
                    // The bindings runtime and the schedule runner deliver the
                    // first turn themselves.
                    first_message: None,
                    task_id: None,
                    provision: true,
                    event_subtype: moatless_types::task::TaskSubType::Open,
                    repository: None,
                },
                move |_task| async move { metadata },
                |_task_id, _first: crate::task::creation::FirstMessage| async move {},
            )
            .await
            .map_err(open_error)?;
        Ok(TaskOpened {
            task_id: outcome.task.task_id,
            provision: match outcome.provision {
                crate::task::creation::ProvisionOutcome::Started
                | crate::task::creation::ProvisionOutcome::Skipped => SandboxProvision::Started,
                crate::task::creation::ProvisionOutcome::Failed => SandboxProvision::Failed,
            },
        })
    }
}

/// The creation failures a Loop can tell apart, with the store failures folded
/// into one. Only `Preflight` is reported to a person; the rest are logged.
fn open_error(error: crate::task::creation::TaskCreationError) -> TaskOpenError {
    use crate::task::creation::TaskCreationError as E;
    match error {
        E::RepoNotFound(id) => TaskOpenError::RepoNotFound(id),
        E::BadRequest(message) => TaskOpenError::BadRequest(message),
        E::Preflight(message) => TaskOpenError::Refused(message),
        E::Store(e) => TaskOpenError::Store(e),
        other => TaskOpenError::Store(StoreError::with_source("open a task", other)),
    }
}

#[async_trait]
impl MessageAcceptor for crate::message::send::MessageSendService {
    async fn accept(&self, turn: InboundTurn) -> Result<(), StoreError> {
        self.send_message_flow(
            &turn.task_id,
            &turn.text,
            None,
            None,
            None,
            &turn.file_ids,
            &[],
            turn.resume,
            moatless_types::task::ExternalOrigin {
                event_id: turn.origin.event_id.as_deref(),
                adapter: turn.origin.adapter.as_deref(),
                subject_key: turn.origin.subject_key.as_deref(),
                source_name: turn.origin.source_name.as_deref(),
                author: turn.origin.author.as_deref(),
                sender_id: turn.origin.sender_id.as_deref(),
                source_task_id: None,
            },
            false,
            turn.sent_by.as_deref(),
            &turn.skills,
            None,
        )
        .await
        .map(|_| ())
        .map_err(|e| {
            StoreError::with_source(format!("accept a turn on task {}", turn.task_id), e)
        })
    }
}

#[async_trait]
impl TaskFinisher for crate::task::closer::TaskCloser {
    async fn close(&self, task_id: &str, reason: CloseReason) -> Result<(), StoreError> {
        // reason: a provider close runs as the system — a webhook carries no
        // caller identity. The existence hiding inside `close_with_event` is
        // what keeps the answer the same for a Task that is not there.
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        self.close_with_event(
            &sys,
            task_id,
            &crate::task::closer::CloseEvent::enriched(reason.actor, reason.payload),
        )
        .await
        .map_err(|e| StoreError::with_source(format!("close task {task_id}"), e))
    }
}

/// The `task_relations` store, narrowed to the one relation a provider records.
pub struct TaskRelations {
    relations: Arc<dyn crate::task::relations::TaskRelationDao>,
}

impl TaskRelations {
    pub fn new(relations: Arc<dyn crate::task::relations::TaskRelationDao>) -> Self {
        Self { relations }
    }
}

#[async_trait]
impl RelationRecorder for TaskRelations {
    async fn record_related_to(&self, from_task_id: &str, related: &RelatedTask) {
        crate::task::relations::record_related_to(self.relations.as_ref(), from_task_id, related)
            .await;
    }
}

/// The `task_activities` table, narrowed to the four reads a read model makes.
///
/// The projection that reads them is `moatless-t3code`; the writes stay behind
/// `task_activities::dao`, where the mapping that produces a row lives.
pub struct Activities {
    pool: moatless_core::db::DbPool,
}

impl Activities {
    pub fn new(pool: moatless_core::db::DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ActivitySource for Activities {
    async fn since(
        &self,
        task_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<Activity>, StoreError> {
        crate::task_activities::dao::for_task_since(&self.pool, task_id, since)
            .await
            .map_err(|e| StoreError::with_source(format!("read activities for {task_id}"), e))
    }

    async fn for_messages(
        &self,
        task_id: &str,
        message_uuids: &[String],
    ) -> Result<Vec<Activity>, StoreError> {
        crate::task_activities::dao::for_messages(&self.pool, task_id, message_uuids)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read activities on {task_id}'s messages"), e)
            })
    }

    async fn for_ids(
        &self,
        task_id: &str,
        activity_ids: &[String],
    ) -> Result<Vec<Activity>, StoreError> {
        crate::task_activities::dao::for_activity_ids(&self.pool, task_id, activity_ids)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read the named activities on {task_id}"), e)
            })
    }

    async fn tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<Activity>, StoreError> {
        crate::task_activities::dao::find_tool_call(&self.pool, task_id, tool_use_id)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read the {tool_use_id} call on {task_id}"), e)
            })
    }
}
