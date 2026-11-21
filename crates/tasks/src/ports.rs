//! Tasks' side of the seams declared in `moatless_ports::task`.
//!
//! Each adapter wraps one store and narrows it to the projection a peer crate
//! reads or the entry it writes, converting `sqlx::Error` into a `StoreError`
//! that boxes it. The composition root builds these once and hands out the port
//! a consumer needs.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use moatless_ports::activity::{ActivitySource, SubagentUsage};
use moatless_ports::identity::UserResolver;
use moatless_ports::task::{
    AssistantReply, CloseReason, EventSubject, ExternalEvent, InboundTurn, MessageAcceptor,
    NewTask, PendingWorkReader, RelationRecorder, SandboxProvision, TagResolver, TaskDescriptor,
    TaskFinisher, TaskOpenError, TaskOpened, TaskOpener, TaskReader, ToolCallRecord,
    TranscriptAppend, TranscriptEntry, TranscriptOrigin, TurnCounter, TurnSignals, TurnStateReader,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::activity::Activity;
use moatless_types::store::StoreError;
use moatless_types::task::{RelatedTask, TaskStatus};

use crate::message::dao::MessageDao;
use crate::tag::dao::TagDao;
use crate::task::dao::TaskDao;
use crate::task_turn::dao::TaskTurnDao;

/// The `tasks` table, narrowed to the four facts a peer reads.
pub struct TaskProjection {
    tasks: Arc<dyn TaskDao>,
}

impl TaskProjection {
    pub(crate) fn new(tasks: Arc<dyn TaskDao>) -> Self {
        Self { tasks }
    }
}

/// The port's view of a Task row.
fn descriptor(task: moatless_types::task::Task) -> TaskDescriptor {
    TaskDescriptor {
        task_id: task.task_id,
        status: task.status,
        name: task.name,
        owner_user_id: task.owner_user_id,
        workspace_id: task.workspace_id,
        created_by: task.created_by,
        reporter_user_id: task.reporter_user_id,
        run_as_user_id: task.run_as_user_id,
        loop_id: task.loop_id,
        ttl_minutes: task.ttl_minutes,
    }
}

fn read_failed(task_id: &str) -> impl FnOnce(sqlx::Error) -> StoreError + '_ {
    move |e| StoreError::with_source(format!("read task {task_id}"), e)
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
            .map_err(read_failed(task_id))?;
        Ok(task.map(descriptor))
    }

    async fn may_access(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, StoreError> {
        Ok(self.readable_status(task_id, ctx).await?.is_some())
    }

    async fn readable_status(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskStatus>, StoreError> {
        self.tasks
            .find_readable_status(task_id, ctx)
            .await
            .map_err(read_failed(task_id))
    }

    async fn written(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskDescriptor>, StoreError> {
        let task = self
            .tasks
            .find_written_by_id(task_id, ctx)
            .await
            .map_err(read_failed(task_id))?;
        Ok(task.map(descriptor))
    }

    async fn owned(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskDescriptor>, StoreError> {
        let task = self
            .tasks
            .find_owned_by_id(task_id, ctx)
            .await
            .map_err(read_failed(task_id))?;
        Ok(task.map(descriptor))
    }
}

/// A Task's conversation, narrowed to the transcript operations a peer
/// performs.
pub struct Transcript {
    messages: Arc<dyn MessageDao>,
}

impl Transcript {
    pub(crate) fn new(messages: Arc<dyn MessageDao>) -> Self {
        Self { messages }
    }
}

#[async_trait]
impl TranscriptAppend for Transcript {
    async fn append(&self, entry: &TranscriptEntry) -> Result<(), StoreError> {
        self.messages
            .append_binding_reply(entry)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("append to task {} transcript", entry.task_id), e)
            })
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

/// The `task_turns` table, narrowed to the two questions peers ask of it: which
/// Turn a row appended now belongs to, and whether one is in flight.
pub struct TurnLedger {
    turns: Arc<dyn TaskTurnDao>,
}

impl TurnLedger {
    pub(crate) fn new(turns: Arc<dyn TaskTurnDao>) -> Self {
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

#[async_trait]
impl TurnStateReader for TurnLedger {
    async fn turn_signals(&self, task_id: &str) -> Result<TurnSignals, StoreError> {
        let signals = self
            .turns
            .latest_running_signals(task_id)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read the running signals of {task_id}"), e)
            })?;
        Ok(TurnSignals {
            turn_status: signals.turn_status,
            turn_ended_at: signals.turn_ended_at,
            latest_user_message_at: signals.latest_user_message_at,
        })
    }
}

/// The `tags` table, narrowed to name-to-id resolution.
pub struct TagCatalog {
    tags: Arc<dyn TagDao>,
}

impl TagCatalog {
    pub(crate) fn new(tags: Arc<dyn TagDao>) -> Self {
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
/// [`TaskCreationService::create`] is generic over two closures tied to the
/// backend's `AppState`, so it is not dyn-safe; a Loop needs neither, so this
/// port is that entry and the inherent method stays for the HTTP path.
#[async_trait]
impl TaskOpener for crate::task::creation::TaskCreationService {
    async fn open(&self, req: NewTask) -> Result<TaskOpened, TaskOpenError> {
        let metadata = req.event_metadata.clone();
        let workspace_id = req.workspace_id;
        // A Loop outlives the Workspace it names, so the liveness `create_loop`
        // checked can be stale by the time this fires. `service::normalize`
        // refuses a tombstone the same way on the HTTP path.
        let workspace = self
            .workspaces
            .resolve(&workspace_id)
            .await?
            .ok_or_else(|| {
                TaskOpenError::BadRequest(format!("workspace {workspace_id} does not exist"))
            })?;
        if workspace.deleted {
            return Err(TaskOpenError::BadRequest(format!(
                "workspace {workspace_id} has been deleted; restore it from Git before firing"
            )));
        }
        let repository_id =
            crate::task::service::primary_repository_id(self.workspaces.as_ref(), &workspace_id)
                .await?;
        let (owner_user_id, run_as_user_id) = owner_and_run_as(
            self.users.as_ref(),
            workspace.run_as_user_id.as_deref(),
            req.created_by.clone(),
            req.run_as_user_id,
        )
        .await?;
        let outcome = self
            .create(
                crate::task::creation::NewTaskRequest {
                    forked_from: None,
                    repository_id,
                    agent_profile: req.agent_profile,
                    name: req.name,
                    description: req.description,
                    branch: None,
                    model: req.model,
                    created_by: req.created_by,
                    reporter_user_id: req.reporter_user_id,
                    owner_user_id,
                    run_as_user_id,
                    headless: true,
                    visibility: req.visibility,
                    loop_id: Some(req.loop_id),
                    workspace_id,
                    // A Loop firing has no sandbox to join: it is triggered by
                    // an inbound event, not by a Task that already has one warm.
                    sandbox_id: None,
                    checkout_mode: None,
                    template_id: None,
                    append_system_prompt: req.append_system_prompt,
                    ttl_minutes: req.ttl_minutes,
                    close_after_settled_days: req.close_after_settled_days,
                    settle_after_hours: req.settle_after_hours,
                    sandbox_placement: crate::task::models::SandboxPlacement {
                        node_selector: req.node_selector,
                        stop_when_done: req.stop_when_done,
                    },
                    tag_ids: req.tag_ids,
                    skills: req.skills,
                    memory_preload: req.memory_preload,
                    agent_properties: req.agent_properties,
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
                |_task_id, _first: crate::task::creation::FirstMessage| async move { Ok(()) },
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

/// The owner and Task-level run-as of a Loop's [`NewTask`].
///
/// The creator owns it, never the reporter: a reporter owning the Task would
/// get owner rights over another user's Loop run, and `access_filter` already
/// lets them read and message it. A Workspace run-as drops the Loop's: a
/// Task-level one would outrank it.
async fn owner_and_run_as(
    users: &dyn UserResolver,
    workspace_run_as: Option<&str>,
    owner_user_id: Option<String>,
    run_as: Option<String>,
) -> Result<(Option<String>, Option<String>), StoreError> {
    let Some(run_as) = run_as else {
        return Ok((owner_user_id, None));
    };
    if workspace_run_as.is_some_and(|user| !user.is_empty()) {
        return Ok((owner_user_id, None));
    }
    let is_bot = users
        .run_as_target(&run_as)
        .await?
        .is_some_and(|user| user.is_bot);
    Ok(if is_bot {
        (owner_user_id, Some(run_as))
    } else {
        (Some(run_as), None)
    })
}

/// The creation failures a Loop can tell apart, with the store failures folded
/// into one. Only `Preflight` is reported to a person; the rest are logged.
fn open_error(error: crate::task::creation::TaskCreationError) -> TaskOpenError {
    use crate::task::creation::TaskCreationError as E;
    match error {
        E::RepoNotFound(id) => TaskOpenError::RepoNotFound(id),
        E::BadRequest(message) => TaskOpenError::BadRequest(message),
        E::Preflight { message, .. } => TaskOpenError::Refused(message),
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
            // `queue`: see `MessageAcceptor`.
            true,
            None,
            None,
        )
        .await
        .map(|_| ())
        .map_err(|e| StoreError::with_source(format!("accept a turn on task {}", turn.task_id), e))
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

#[async_trait]
impl RelationRecorder for crate::task::relations::TaskRelations {
    async fn record_related_to(&self, from_task_id: &str, related: &RelatedTask) {
        crate::task::relations::TaskRelations::record_related_to(self, from_task_id, related).await;
    }
}

/// The unclaimed-user-message predicate, answered for a Sandbox's members.
///
/// The control plane holds the wake signal but not the queue it stands for;
/// this is the read that turns "this Sandbox has work" into the Tasks to
/// notify.
pub struct PendingWork {
    pool: moatless_core::db::DbPool,
}

impl PendingWork {
    pub(crate) fn new(pool: moatless_core::db::DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PendingWorkReader for PendingWork {
    async fn members_awaiting_dispatch(&self, task_id: &str) -> Result<Vec<String>, StoreError> {
        crate::inbox::dao::members_awaiting_dispatch(&self.pool, task_id)
            .await
            .map_err(|e| StoreError::with_source(format!("read pending members for {task_id}"), e))
    }
}

/// The `conversation_activities` table, narrowed to the four reads a read
/// model makes. The projection that reads them is `moatless-t3code`; the
/// Projector is the only writer.
pub struct Activities {
    pool: moatless_core::db::DbPool,
}

impl Activities {
    pub(crate) fn new(pool: moatless_core::db::DbPool) -> Self {
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
        crate::activities::dao::for_task_since(&self.pool, task_id, since)
            .await
            .map_err(|e| StoreError::with_source(format!("read activities for {task_id}"), e))
    }

    async fn for_messages(
        &self,
        task_id: &str,
        message_uuids: &[String],
    ) -> Result<Vec<Activity>, StoreError> {
        crate::activities::dao::for_messages(&self.pool, task_id, message_uuids)
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
        crate::activities::dao::for_activity_ids(&self.pool, task_id, activity_ids)
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
        crate::activities::dao::find_tool_call(&self.pool, task_id, tool_use_id)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read the {tool_use_id} call on {task_id}"), e)
            })
    }

    async fn settled_calls(
        &self,
        task_id: &str,
        tool_use_ids: &[String],
    ) -> Result<HashSet<String>, StoreError> {
        crate::activities::dao::settled_calls(&self.pool, task_id, tool_use_ids)
            .await
            .map(|settled| settled.into_iter().collect())
            .map_err(|e| StoreError::with_source(format!("read settled calls on {task_id}"), e))
    }

    async fn subagent_usage(
        &self,
        task_id: &str,
        calls: &[String],
    ) -> Result<Vec<SubagentUsage>, StoreError> {
        crate::conversation::read_dao::subagent_usage(&self.pool, task_id, calls)
            .await
            .map_err(|e| StoreError::with_source(format!("read subagent usage on {task_id}"), e))
    }
}

#[cfg(test)]
mod tests {
    use moatless_ports::identity::{MockUserResolver, RunAsUser};
    use uuid::Uuid;

    use super::owner_and_run_as;

    const LOOP_USER: &str = "6f1c1f0e-0000-4000-8000-000000000001";
    const CREATOR: &str = "creator";

    fn loop_user(is_bot: bool) -> MockUserResolver {
        let mut users = MockUserResolver::new();
        users.expect_run_as_target().returning(move |_| {
            Ok(Some(RunAsUser {
                id: Uuid::parse_str(LOOP_USER).unwrap(),
                login: "loop-user".to_string(),
                is_bot,
            }))
        });
        users
    }

    async fn split(
        users: &MockUserResolver,
        workspace_run_as: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        owner_and_run_as(
            users,
            workspace_run_as,
            Some(CREATOR.to_string()),
            Some(LOOP_USER.to_string()),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn the_creator_owns_a_task_that_runs_as_the_loops_bot() {
        assert_eq!(
            split(&loop_user(true), None).await,
            (Some(CREATOR.to_string()), Some(LOOP_USER.to_string()))
        );
    }

    #[tokio::test]
    async fn a_human_loop_user_keeps_the_task_it_runs_as() {
        assert_eq!(
            split(&loop_user(false), None).await,
            (Some(LOOP_USER.to_string()), None)
        );
    }

    #[tokio::test]
    async fn a_workspace_run_as_outranks_the_loops() {
        assert_eq!(
            split(&MockUserResolver::new(), Some("machine")).await,
            (Some(CREATOR.to_string()), None)
        );
    }
}
