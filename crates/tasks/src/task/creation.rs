//! Task creation service — the single owner of the create-then-provision
//! sequence: short id, preflight, insert, tags, relations, optional first
//! message, provisioning, and the task event.
//!
//! [`TaskCreationService::create`] is every caller's one entry point. It
//! guarantees a failed preflight or provisioning step still records the task as
//! `Error` with a `SandboxStatus::Error` event.

use std::future::Future;
use std::sync::Arc;

use moatless_ports::identity::UserResolver;
use moatless_ports::sandbox::{PreflightPort, SandboxError, SandboxJoiner, SandboxStarter};
use moatless_types::access_control::ScopeContext;
use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use moatless_types::task::TaskSubType;

use crate::message::error::MessageError;
use crate::tag::dao::TagDao;
use crate::task::dao::TaskDao;
use crate::task::error::TaskError;
use crate::task::fork::TaskForkDao;
use crate::task::models::TaskFork;
use crate::task::models::{Task, TaskStatus};
use crate::task::relations::{
    CreatedByParent, TaskRelationDao, record_created_by, record_forked_from, record_related_to,
};
use crate::task::repositories::{TaskRepositoryDao, TaskRepositorySource};
use crate::task::schemas::MessageContextInput;
use moatless_core::events::EventPublisher;
use moatless_types::agent::{AgentMode, AgentProfile};
use moatless_types::task::RelatedTask;
use moatless_types::task::Visibility;
use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};
use moatless_workspaces::repository::models::Repository;

/// `lifecycleEvent` of the Sandbox error published when preflight refuses a task.
pub const PREFLIGHT_FAILED: &str = "preflight_failed";
/// `lifecycleEvent` of the Sandbox error published when a new task's first
/// message cannot be delivered.
pub const FIRST_MESSAGE_FAILED: &str = "first_message_failed";
/// The `lifecycleEvent`s published only after the task was marked `Error`, which
/// the Sandbox error handler must leave in place rather than reopen.
pub const TASK_ERRORED_LIFECYCLE_EVENTS: &[&str] = &[PREFLIGHT_FAILED, FIRST_MESSAGE_FAILED];

/// Normalised task-creation input; every call path translates its domain input
/// into this.
pub struct NewTaskRequest {
    /// The Workspace's primary repository; `None` when it places none.
    pub repository_id: Option<String>,
    pub agent_profile: AgentProfile,
    pub name: Option<String>,
    pub description: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub created_by: Option<String>,
    /// See [`Task::reporter_user_id`].
    pub reporter_user_id: Option<String>,
    /// Authorized owner selected by `TaskOpener::open`; `None` uses `created_by`.
    /// A reporter may own the Task under a bot or Workspace run-as.
    /// HTTP normalization leaves this unset.
    pub owner_user_id: Option<String>,
    /// See [`moatless_types::task::Task::run_as_user_id`]. Already authorized
    /// by the time it reaches here: by `ScopeContext::can_run_as` in
    /// `service::normalize`, or on a Loop's path by the admin who set its run-as.
    pub run_as_user_id: Option<String>,
    pub headless: bool,
    pub visibility: Visibility,
    pub loop_id: Option<String>,
    /// The Workspace the Task runs in. Every Task has one — a caller that names
    /// none is resolved to one, or refused, before it reaches here.
    pub workspace_id: String,
    pub sandbox_id: Option<String>,
    pub checkout_mode: Option<moatless_types::task::CheckoutMode>,
    pub template_id: Option<String>,
    pub append_system_prompt: Option<String>,
    pub ttl_minutes: i32,
    pub close_after_settled_days: i32,
    pub settle_after_hours: Option<i32>,
    /// What the firing Loop imposes on the Sandbox row written alongside the
    /// Task. Default on every path but a Loop firing.
    pub sandbox_placement: crate::task::models::SandboxPlacement,
    /// Pre-resolved tag ids to attach (adapters pass their Loop's tag ids; the
    /// HTTP path passes none).
    pub tag_ids: Vec<String>,
    /// Skills every fresh agent session on the task activates — a Loop's
    /// `skills`, or those chosen when the task was created.
    pub skills: Vec<String>,
    /// See [`Task::memory_preload`].
    pub memory_preload: Vec<String>,
    /// See [`Task::agent_properties`].
    pub agent_properties: moatless_types::agent::AgentProperties,
    /// Optional `created_by` parent relation to record after insert. A child
    /// joining its parent's Sandbox is also cut at the parent's head; see
    /// `TaskCreationService::continue_parent_session`.
    pub parent: Option<CreatedByParent>,
    /// Set when this Task is a fork. Records the `forked_from` relation, and
    /// the `task_forks` row when the parent had a session to branch — both
    /// written after insert, before the first message reads them.
    pub forked_from: Option<ForkedFrom>,
    /// Optional `related_to` peer relation to record after insert.
    pub related_to: Option<RelatedTask>,
    /// Optional first message; when present the service invokes the caller's
    /// send callback before provisioning, and a send that fails takes the whole
    /// create down with it — see [`TaskCreationError::FirstMessage`].
    pub first_message: Option<FirstMessage>,
    /// The task's id, when the caller owns it.
    ///
    /// T3 pre-mints it before create; others pass `None`, get a fresh uuid, and
    /// a collision fails the insert.
    pub task_id: Option<String>,
    /// Whether to start a sandbox as part of creation. `false` only from the T3
    /// compatibility surface, whose client can create a thread before anyone
    /// has typed into it.
    pub provision: bool,
    /// Task-event subtype to publish — `Created` on the HTTP path, `Open` on
    /// loop/adapter paths. Kept per-caller by design.
    pub event_subtype: TaskSubType,
    /// Optionally pre-fetched repository. `Some` skips the service's own lookup
    /// (already done for access control); `None` fetches `repository_id`, if
    /// any, surfacing [`TaskCreationError::RepoNotFound`] if unknown.
    pub repository: Option<Repository>,
}

/// What makes a Task a fork, carried from the fork handler to the one place
/// that may write it — after the Task row exists, and before its first message.
pub struct ForkedFrom {
    pub side_chat: bool,
    pub source_task_id: String,
    pub at_turn: Option<i32>,
    /// The cut as a trajectory event; `None` when the parent recorded nothing
    /// at or before it, and the fork's first turn starts a fresh session.
    pub source_event_id: Option<uuid::Uuid>,
}

/// The first message payload the HTTP path sends through `send_message_flow`.
/// Carried in the request but delivered via the caller's callback, since
/// message assembly needs the full `AppState`.
pub struct FirstMessage {
    pub text: String,
    pub model: Option<String>,
    pub agent_mode: Option<AgentMode>,
    /// Reasoning effort for the first turn, in the agent's own vocabulary.
    pub effort: Option<String>,
    pub file_ids: Vec<String>,
    pub contexts: Vec<MessageContextInput>,
    pub sent_by: Option<String>,
    /// The message's uuid when the caller owns it — see
    /// [`crate::message::send::MessageSendService::send_message_flow`].
    pub message_uuid: Option<String>,
    /// Skills to activate for this first turn.
    pub skills: Vec<String>,
    /// The `created_by` parent, which the first turn then reports to.
    pub source_task_id: Option<String>,
}

/// Whether the best-effort provision step started the sandbox. The Loop
/// `schedule_runner` records a `provision_failed` metric off this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionOutcome {
    Started,
    Failed,
    /// No sandbox was asked for — only the T3 compatibility surface reaches
    /// this, whose client creates a thread before typing. The task is ordinary
    /// and open, needing no new status.
    Skipped,
}

/// Result of a successful creation: the inserted row plus the provision result.
/// `task` is as inserted — DB-populated columns like `updated_at` are `None`;
/// re-fetch by id if needed.
#[derive(Debug)]
pub struct TaskCreationOutcome {
    pub task: Task,
    pub provision: ProvisionOutcome,
}

/// Failure modes surfaced to the caller. The HTTP handler maps these to status
/// codes (`Preflight` → 403, `RepoNotFound` → 404, the rest → 500).
#[derive(Debug, thiserror::Error)]
pub enum TaskCreationError {
    #[error("repository {0} not found")]
    RepoNotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("failed to join sandbox: {0}")]
    SandboxJoin(String),
    #[error("failed to join sandbox and cleanup inserted task: join={join}; cleanup={cleanup}")]
    SandboxJoinCleanup { join: String, cleanup: String },
    /// The first message was refused or could not be persisted. The inserted
    /// Task is deleted before this is returned — it had no brief to run.
    #[error(transparent)]
    FirstMessage(MessageError),
    #[error(
        "failed to send first message and cleanup inserted task: send={message}; cleanup={cleanup}"
    )]
    FirstMessageCleanup { message: String, cleanup: String },
    /// See [`PreflightRejection::throttled`](moatless_ports::sandbox::PreflightRejection::throttled).
    #[error("{message}")]
    Preflight { message: String, throttled: bool },
    #[error("failed to generate a unique short id")]
    ShortId,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    /// A published store seam failed. Distinct from [`Self::Db`] only in where
    /// the failure was raised.
    #[error(transparent)]
    Store(#[from] moatless_types::store::StoreError),
}

impl From<TaskCreationError> for TaskError {
    fn from(e: TaskCreationError) -> Self {
        match e {
            TaskCreationError::RepoNotFound(id) => TaskError::RepoNotFound(id),
            TaskCreationError::BadRequest(msg) => TaskError::BadRequest(msg),
            TaskCreationError::SandboxJoin(msg) => TaskError::Internal(msg),
            TaskCreationError::SandboxJoinCleanup { join, cleanup } => {
                TaskError::Internal(format!("{join}; cleanup failed: {cleanup}"))
            }
            TaskCreationError::FirstMessage(e) => TaskError::FirstMessage(e),
            TaskCreationError::FirstMessageCleanup { message, cleanup } => {
                TaskError::Internal(format!("{message}; cleanup failed: {cleanup}"))
            }
            TaskCreationError::Preflight {
                message,
                throttled: true,
            } => TaskError::PreflightThrottled(message),
            TaskCreationError::Preflight { message, .. } => TaskError::Preflight(message),
            TaskCreationError::ShortId => {
                TaskError::Internal("Failed to generate unique short ID".to_string())
            }
            TaskCreationError::Db(e) => TaskError::Db(e),
            TaskCreationError::Store(e) => TaskError::Store(e),
        }
    }
}

/// Owns the create-then-provision sequence. Constructed from mockable
/// dependencies only — no `&AppState` anywhere — so it is unit-testable without
/// a database.
pub struct TaskCreationService {
    tasks: Arc<dyn TaskDao>,
    tags: Arc<dyn TagDao>,
    relations: Arc<dyn TaskRelationDao>,
    task_repositories: Arc<dyn TaskRepositoryDao>,
    forks: Arc<dyn TaskForkDao>,
    repositories: Arc<dyn RepositoryReader>,
    /// Read by the `TaskOpener` port, which resolves a Loop's Workspace before
    /// it builds a [`NewTaskRequest`].
    pub(crate) workspaces: Arc<dyn WorkspaceResolver>,
    /// Read by the `TaskOpener` port, which hands a requester the Task only
    /// when the Loop runs as a bot.
    pub(crate) users: Arc<dyn UserResolver>,
    events: Option<Arc<dyn EventPublisher>>,
    starter: Arc<dyn SandboxStarter>,
    joiner: Arc<dyn SandboxJoiner>,
    preflight: Arc<dyn PreflightPort>,
}

impl TaskCreationService {
    // reason: independent injected collaborators (five DAOs, the user lookup,
    // the event publisher, the lifecycle seams, the preflight seam); bundling them behind
    // a one-use struct only shuffles the same fields.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        tasks: Arc<dyn TaskDao>,
        tags: Arc<dyn TagDao>,
        relations: Arc<dyn TaskRelationDao>,
        task_repositories: Arc<dyn TaskRepositoryDao>,
        forks: Arc<dyn TaskForkDao>,
        repositories: Arc<dyn RepositoryReader>,
        workspaces: Arc<dyn WorkspaceResolver>,
        users: Arc<dyn UserResolver>,
        events: Option<Arc<dyn EventPublisher>>,
        starter: Arc<dyn SandboxStarter>,
        joiner: Arc<dyn SandboxJoiner>,
        preflight: Arc<dyn PreflightPort>,
    ) -> Self {
        Self {
            tasks,
            tags,
            relations,
            task_repositories,
            forks,
            repositories,
            workspaces,
            users,
            events,
            starter,
            joiner,
            preflight,
        }
    }

    /// Runs the create-then-provision sequence documented on the module.
    /// `build_event_payload` and `send_first_message` are callbacks closing
    /// over the caller's `AppState`, which the service cannot hold.
    pub async fn create<EvFut, MsgFut>(
        &self,
        mut req: NewTaskRequest,
        build_event_payload: impl FnOnce(Task) -> EvFut + Send,
        send_first_message: impl FnOnce(String, FirstMessage) -> MsgFut + Send,
    ) -> Result<TaskCreationOutcome, TaskCreationError>
    where
        EvFut: Future<Output = serde_json::Value> + Send,
        MsgFut: Future<Output = Result<(), MessageError>> + Send,
    {
        let task_id = req
            .task_id
            .take()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if req.sandbox_id.is_none() && req.checkout_mode.is_some() {
            return Err(TaskCreationError::BadRequest(
                "checkout_mode requires sandbox_id".to_string(),
            ));
        }
        let join_checkout_mode = req.sandbox_id.as_ref().map(|_| {
            req.checkout_mode
                .unwrap_or(moatless_types::task::CheckoutMode::Shared)
        });
        // Every new row names its mode, so the column can go NOT NULL.
        let persisted_checkout_mode = Some(
            join_checkout_mode
                .or(req.checkout_mode)
                .unwrap_or(moatless_types::task::CheckoutMode::Shared),
        );
        let short_id = self.generate_unique_short_id().await?;
        let now = chrono::Utc::now();
        let owner_user_id = req.owner_user_id.clone().or_else(|| req.created_by.clone());

        let mut row = Task {
            task_id: task_id.clone(),
            short_id: Some(short_id),
            created_at: now,
            last_message_at: now,
            agent_profile: req.agent_profile,
            repository_id: req.repository_id.clone(),
            status: TaskStatus::Open,
            total_cost_usd: 0.0,
            template_id: req.template_id.clone(),
            name: req.name.clone(),
            description: req.description.clone(),
            branch: req.branch.clone(),
            model: req.model.clone(),
            created_by: req.created_by.clone(),
            reporter_user_id: req.reporter_user_id.clone(),
            visibility: req.visibility,
            owner_user_id,
            run_as_user_id: req.run_as_user_id.clone(),
            updated_at: None,
            append_system_prompt: req.append_system_prompt.clone(),
            headless: req.headless,
            ttl_minutes: req.ttl_minutes,
            close_after_settled_days: req.close_after_settled_days,
            settle_after_hours: req.settle_after_hours,
            loop_id: req.loop_id.clone(),
            workspace_id: Some(req.workspace_id.clone()),
            sandbox_id: req.sandbox_id.clone(),
            checkout_mode: persisted_checkout_mode,
            skills: req.skills.clone(),
            memory_preload: req.memory_preload.clone(),
            agent_properties: req.agent_properties.clone(),
        };

        // Repo is needed for preflight. Reuse a caller-supplied one if present
        // (the HTTP path already fetched it); otherwise look it up, which also
        // yields the 404 for an unknown id.
        let repo = match (req.repository.take(), &req.repository_id) {
            (Some(repo), _) => Some(repo),
            (None, None) => None,
            (None, Some(repository_id)) => {
                // reason: non-HTTP creation paths (adapters, loops) resolve
                // the repository configured on the Loop, not user input; the
                // HTTP path passes an already access-checked repository.
                #[allow(clippy::disallowed_methods)]
                let sys_ctx = ScopeContext::system();
                Some(
                    self.repositories
                        .describe(&sys_ctx, repository_id)
                        .await?
                        .ok_or_else(|| TaskCreationError::RepoNotFound(repository_id.clone()))?,
                )
            }
        };

        // Preflight. On failure persist the row as `Error` and publish a
        // sandbox error event (the `start_task` pattern) so the task surfaces as
        // errored rather than silently proceeding; then surface the typed error.
        if let Err(e) = self.preflight.validate(&row, repo.as_ref()).await {
            tracing::warn!(task_id = %task_id, error = %e, "Task creation preflight failed");
            row.status = TaskStatus::Error;
            self.tasks.insert_task(&row, &req.sandbox_placement).await?;
            self.publish_sandbox_error(&task_id, &e.user_message, &e.detail, PREFLIGHT_FAILED)
                .await;
            return Err(TaskCreationError::Preflight {
                message: e.user_message,
                throttled: e.throttled,
            });
        }

        self.tasks.insert_task(&row, &req.sandbox_placement).await?;
        // `insert_task` writes the Sandbox row for a request that named none,
        // under the task's own id. Carry it back onto the in-memory row: the
        // create response and the `task.created` event are built from this row,
        // not re-read.
        row.sandbox_id.get_or_insert_with(|| task_id.clone());
        tracing::info!(
            task_id = %task_id,
            created_by = ?row.created_by,
            repository_id = ?row.repository_id,
            workspace_id = ?row.workspace_id,
            "Task created"
        );

        self.record_repositories(&task_id, &req).await?;

        // Tags (best-effort).
        if !req.tag_ids.is_empty() {
            if let Err(e) = self.tags.add_task_tags(&task_id, &req.tag_ids).await {
                tracing::warn!(task_id = %task_id, error = %e, "Failed to assign tags to task");
            }
        }

        // Relations (best-effort; helpers swallow-and-log).
        if let Some(parent) = &req.parent {
            record_created_by(self.relations.as_ref(), &task_id, parent).await;
        }
        if let Some(related) = &req.related_to {
            record_related_to(self.relations.as_ref(), &task_id, related).await;
        }

        // The fork's edge and its anchors, which are not best-effort: a fork
        // whose edge is missing is an orphan holding a conversation it cannot
        // explain, and one whose anchors are missing silently starts its first
        // turn on an empty session. Both are written before the first message,
        // because that message is what the pod acts on.
        if let Some(fork) = &req.forked_from {
            record_forked_from(
                self.relations.as_ref(),
                &task_id,
                &fork.source_task_id,
                fork.at_turn,
                fork.side_chat,
            )
            .await?;
            if let Some(source_event_id) = fork.source_event_id {
                self.forks
                    .insert(&TaskFork {
                        task_id: task_id.clone(),
                        source_task_id: fork.source_task_id.clone(),
                        at_turn: fork.at_turn,
                        source_event_id,
                    })
                    .await?;
            }
        }
        if req.forked_from.is_none() {
            self.continue_parent_session(&task_id, &req).await?;
        }

        // Join before delivering the first message: the membership row must
        // exist first so the sandbox-bound token can authorize later RPCs.
        // A `NotReady` join (shared sandbox idle-stopped) isn't a create
        // failure — starting the sandbox below re-registers every member.
        // Any other join error is fatal and rolls the inserted task back.
        let mut join_deferred_to_start = false;
        if let Some(checkout_mode) = join_checkout_mode {
            match self
                .joiner
                .join_task(&task_id, checkout_mode, &row.agent_profile)
                .await
            {
                Ok(()) => {}
                Err(SandboxError::NotReady(reason)) => {
                    tracing::info!(
                        task_id = %task_id,
                        sandbox_id = ?row.sandbox_id,
                        checkout_mode = checkout_mode.as_str(),
                        reason = %reason,
                        "Shared sandbox not ready at join; starting it and deferring registration to its init"
                    );
                    join_deferred_to_start = true;
                }
                Err(error) => {
                    tracing::warn!(
                        task_id = %task_id,
                        sandbox_id = ?row.sandbox_id,
                        checkout_mode = checkout_mode.as_str(),
                        error = %error,
                        "Sandbox join failed during task creation"
                    );
                    let join_error = map_join_error(error);
                    if let Err(cleanup_error) = self.tasks.delete_inserted_task(&task_id).await {
                        tracing::error!(
                            task_id = %task_id,
                            sandbox_id = ?row.sandbox_id,
                            error = %cleanup_error,
                            "Failed to cleanup task after sandbox join failure"
                        );
                        return Err(TaskCreationError::SandboxJoinCleanup {
                            join: join_error.to_string(),
                            cleanup: cleanup_error.to_string(),
                        });
                    }
                    return Err(join_error);
                }
            }
        }

        // Persist the first message before provisioning, so the sandbox's
        // startup dispatch can't race ahead and find no pending input.
        //
        // A Task whose brief didn't persist has no second chance to deliver
        // it, so create fails rather than provisioning one — see
        // [`Self::first_message_failed`] for what it leaves behind.
        if let Some(first_message) = req.first_message {
            if let Err(error) = send_first_message(task_id.clone(), first_message).await {
                tracing::warn!(
                    task_id = %task_id,
                    error = %moatless_core::error::causal_chain(&error),
                    "First message failed during task creation"
                );
                // A join that deferred registered nothing on a pod, so it holds
                // the insert down no more than no join at all does.
                let join_landed = join_checkout_mode.is_some() && !join_deferred_to_start;
                return Err(self
                    .first_message_failed(&task_id, error, join_landed)
                    .await);
            }
        }

        // Provision is best-effort: failure publishes a sandbox error event,
        // but creation still succeeds. `req.provision` is unrelated to
        // `first_message` (bindings/schedule runner send `None` but still
        // provision). A landed join needs no start; a deferred one wakes
        // the shared sandbox via this task's `sandbox_id`.
        let provision = if join_checkout_mode.is_some() && !join_deferred_to_start {
            ProvisionOutcome::Started
        } else if (join_checkout_mode.is_some() && join_deferred_to_start) || req.provision {
            match self.starter.start(&task_id).await {
                Ok(()) => ProvisionOutcome::Started,
                Err(e) => {
                    tracing::warn!(task_id = %task_id, error = %e, "Sandbox start failed during task creation");
                    self.publish_sandbox_error(
                        &task_id,
                        "Sandbox provisioning failed during task creation.",
                        &e.to_string(),
                        "provision_failed",
                    )
                    .await;
                    ProvisionOutcome::Failed
                }
            }
        } else {
            ProvisionOutcome::Skipped
        };

        if let Some(events) = &self.events {
            let payload = build_event_payload(row.clone()).await;
            events
                .publish_task_event(
                    &task_id,
                    req.event_subtype,
                    Some(row.agent_profile.as_str()),
                    payload,
                )
                .await;
        }

        Ok(TaskCreationOutcome {
            task: row,
            provision,
        })
    }

    /// Gives the new Task its own repository and its Workspace's, and a fork
    /// its source Task's first, so an inherited row keeps its `source`. Not
    /// best-effort: `reach` authorizes reading a pull request through them.
    async fn record_repositories(
        &self,
        task_id: &str,
        req: &NewTaskRequest,
    ) -> Result<(), TaskCreationError> {
        if let Some(fork) = &req.forked_from {
            for inherited in self.task_repositories.list(&fork.source_task_id).await? {
                self.task_repositories
                    .record(task_id, &[inherited.repository_id], inherited.source)
                    .await?;
            }
        }
        let mut placed: Vec<String> = self
            .workspaces
            .placements(&req.workspace_id)
            .await?
            .into_iter()
            .map(|placement| placement.repository_id)
            .collect();
        if let Some(repository_id) = &req.repository_id {
            if !placed.contains(repository_id) {
                placed.push(repository_id.clone());
            }
        }
        self.task_repositories
            .record(task_id, &placed, TaskRepositorySource::Workspace)
            .await?;
        Ok(())
    }

    /// Cuts a `created_by` child joining its parent's Sandbox at the parent's
    /// trajectory head, as a fork. Not best-effort: without the cut, a Claude
    /// child's first turn runs `--continue` in the shared project dir, resumes
    /// the parent's session, and every append it makes is refused as foreign.
    async fn continue_parent_session(
        &self,
        task_id: &str,
        req: &NewTaskRequest,
    ) -> Result<(), TaskCreationError> {
        let (Some(parent), Some(sandbox_id)) = (&req.parent, &req.sandbox_id) else {
            return Ok(());
        };
        // reason: the parent is read for its Sandbox alone, and the caller
        // already chose to join that Sandbox under its own access check.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let parent_sandbox = self
            .tasks
            .find_by_id(&parent.parent_task_id, &sys_ctx)
            .await?
            .and_then(|parent| parent.sandbox_id);
        if parent_sandbox.as_deref() != Some(sandbox_id.as_str()) {
            return Ok(());
        }
        let cut = self
            .forks
            .insert_at_head(task_id, &parent.parent_task_id)
            .await?;
        tracing::info!(
            task_id = %task_id,
            parent_task_id = %parent.parent_task_id,
            cut,
            "Child joining its parent's sandbox continues the parent's session"
        );
        Ok(())
    }

    /// Single retry-loop short-id generator shared by every creation path.
    async fn generate_unique_short_id(&self) -> Result<String, TaskCreationError> {
        for _ in 0..10 {
            let id = nanoid::nanoid!(8, &nanoid::alphabet::SAFE);
            if !self.tasks.exists_by_short_id(&id).await? {
                return Ok(id);
            }
        }
        Err(TaskCreationError::ShortId)
    }

    /// Undo a Task whose first message never landed, or record it as `Error`.
    ///
    /// Deleting the row is only honest when the send wrote nothing and no pod
    /// registered this Task: the inbox has no foreign key to `tasks`, and
    /// a shared-sandbox join has no undo. Anything left behind names a Task
    /// that survives the failed create, errored and unprovisioned.
    async fn first_message_failed(
        &self,
        task_id: &str,
        error: MessageError,
        joined: bool,
    ) -> TaskCreationError {
        if !joined && error.wrote_nothing() {
            if let Err(cleanup_error) = self.tasks.delete_inserted_task(task_id).await {
                tracing::error!(
                    task_id = %task_id,
                    error = %cleanup_error,
                    "Failed to cleanup task after first-message failure"
                );
                return TaskCreationError::FirstMessageCleanup {
                    message: error.to_string(),
                    cleanup: cleanup_error.to_string(),
                };
            }
            return TaskCreationError::FirstMessage(error);
        }

        // reason: creation owns this status transition; the caller's authority
        // was already checked to reach here.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        if let Err(status_error) = self
            .tasks
            .update_status_if_not_closed(task_id, TaskStatus::Error, &sys_ctx)
            .await
        {
            tracing::error!(
                task_id = %task_id,
                error = %status_error,
                "Failed to mark task errored after first-message failure"
            );
        }
        self.publish_sandbox_error(
            task_id,
            "The task's first message could not be delivered",
            &moatless_core::error::causal_chain(&error),
            FIRST_MESSAGE_FAILED,
        )
        .await;
        TaskCreationError::FirstMessage(error)
    }

    async fn publish_sandbox_error(
        &self,
        task_id: &str,
        message: &str,
        error: &str,
        lifecycle_event: &str,
    ) {
        if let Some(events) = &self.events {
            events
                .publish_sandbox_event_with_payload(
                    task_id,
                    SandboxStatus::Error,
                    &SandboxEventPayload {
                        message: message.to_string(),
                        error: Some(error.to_string()),
                        lifecycle_event: Some(lifecycle_event.to_string()),
                        ..Default::default()
                    },
                )
                .await;
        }
    }
}

fn map_join_error(error: SandboxError) -> TaskCreationError {
    match error {
        SandboxError::BadRequest(message) => TaskCreationError::BadRequest(message),
        SandboxError::NotFound(message) => {
            TaskCreationError::BadRequest(format!("sandbox not found for join: {message}"))
        }
        other => TaskCreationError::SandboxJoin(other.to_string()),
    }
}

#[cfg(test)]
#[path = "creation_tests.rs"]
mod tests;
