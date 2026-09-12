//! Task creation service — the single owner of the create-then-provision
//! sequence.
//!
//! Creating a task means: generate a unique short id, run the GitHub-token
//! preflight, insert the task row, assign tags, record relations, send an
//! optional first message, provision the sandbox, and publish a task event.
//! [`TaskCreationService`] is the one entry point for that sequence — the HTTP
//! handler and the loop/adapter paths all go through it — so a provisioning fix
//! lands once.
//!
//! Callers normalise their domain input (HTTP payload, webhook, loop config)
//! into a [`NewTaskRequest`] and hand the service the two `AppState`-coupled
//! steps as callbacks (first-message delivery and success-event enrichment).
//! The service owns *when* each step runs and, crucially, the two consistency
//! guarantees the consolidation exists for:
//!
//! * **preflight everywhere** — a failed preflight records the task as `Error`
//!   and publishes a `SandboxStatus::Error` event (mirroring `start_task`)
//!   rather than letting a doomed task proceed to a dead sandbox;
//! * **provision-failure events** — a failed `sandbox.start` publishes a
//!   `SandboxStatus::Error` event so the task shows as errored instead of
//!   silently sitting dead, while creation itself still succeeds;
//! * **one scope rule** — every task persists as `Scope::User` owned by
//!   `created_by`, derived here rather than accepted from the caller.

use std::future::Future;
use std::sync::Arc;

use moatless_ports::sandbox::{PreflightPort, SandboxError, SandboxJoiner, SandboxStarter};
use moatless_types::access_control::{Scope, ScopeContext};
use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use moatless_types::task::TaskSubType;

use crate::tag::dao::TagDao;
use crate::task::dao::TaskDao;
use crate::task::error::TaskError;
use crate::task::fork::{ForkSession, TaskForkDao};
use crate::task::models::TaskFork;
use crate::task::models::{Task, TaskStatus};
use crate::task::relations::{
    CreatedByParent, TaskRelationDao, record_created_by, record_forked_from, record_related_to,
};
use crate::task::schemas::MessageContextInput;
use moatless_core::events::EventPublisher;
use moatless_ports::workspace::{RepositoryReader, WorkspaceResolver};
use moatless_types::agent::{AgentMode, AgentType};
use moatless_types::task::RelatedTask;
use moatless_types::task::Visibility;
use moatless_workspaces::repository::models::Repository;

/// Normalised task-creation input. Every call path (HTTP handler, loops,
/// adapters) translates its domain input into this; the service never inspects
/// HTTP payloads, webhooks, or loop configs.
///
/// Scope and ownership are deliberately absent: the service derives them from
/// `created_by` (see [`TaskCreationService::create`]), so no caller can mint a
/// task its own sandbox cannot control.
pub struct NewTaskRequest {
    pub repository_id: String,
    pub agent_type: AgentType,
    pub name: Option<String>,
    pub description: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub created_by: Option<String>,
    /// See [`moatless_types::task::Task::run_as_user_id`]. Already authorized
    /// by the time it reaches here — `service::normalize` is where the
    /// `ScopeContext::can_run_as` check lives.
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
    pub vibe: Option<bool>,
    pub append_system_prompt: Option<String>,
    pub ttl_minutes: i32,
    pub inactive_ttl_days: i32,
    /// Pre-resolved tag ids to attach (adapters pass their Loop's tag ids;
    /// the HTTP path passes none).
    pub tag_ids: Vec<String>,
    /// Skills every fresh agent session on the task activates — a Loop's
    /// `skills`, or those chosen when the task was created.
    pub skills: Vec<String>,
    /// Optional `created_by` parent relation to record after insert.
    pub parent: Option<CreatedByParent>,
    /// Set when this Task is a fork. Records the `forked_from` relation and,
    /// when the parent's agent had a session to branch, the `task_forks` row —
    /// both after the insert the two reference, and both before the first
    /// message, which is what the pod reads them for.
    pub forked_from: Option<ForkedFrom>,
    /// Optional `related_to` peer relation to record after insert.
    pub related_to: Option<RelatedTask>,
    /// Optional first message; when present the service invokes the caller's
    /// send callback after provisioning.
    pub first_message: Option<FirstMessage>,
    /// The task's id, when the caller owns it. The T3 compatibility surface
    /// navigates to a thread id it minted before the create reaches us, so a
    /// server-minted id would strand the client on a thread that never
    /// appears. Every other caller passes `None` and gets a fresh uuid; a
    /// collision fails the insert rather than silently reminting.
    pub task_id: Option<String>,
    /// Whether to start a sandbox as part of creation. `false` only from the
    /// T3 compatibility surface, whose client can create a thread before
    /// anyone has typed into it.
    pub provision: bool,
    /// Task-event subtype to publish — `Created` on the HTTP path, `Open` on
    /// loop/adapter paths. Kept per-caller by design.
    pub event_subtype: TaskSubType,
    /// Optionally pre-fetched repository. When `Some`, the service uses it and
    /// skips its own lookup (the HTTP path already fetched the repo for access
    /// control); when `None`, the service fetches it and surfaces
    /// [`TaskCreationError::RepoNotFound`] for an unknown id.
    pub repository: Option<Repository>,
}

/// What makes a Task a fork, carried from the fork handler to the one place
/// that may write it — after the Task row exists, and before its first message.
pub struct ForkedFrom {
    pub source_task_id: String,
    pub at_turn: Option<i32>,
    /// `None` when the parent's agent never ran at or before the cut. The fork
    /// still carries the conversation; its first turn starts on a fresh session.
    pub session: Option<ForkSession>,
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
}

/// Whether the best-effort provision step started the sandbox. Ticket 02's
/// `schedule_runner` records a `provision_failed` metric off this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionOutcome {
    Started,
    Failed,
    /// No sandbox was asked for, because the task was created with nothing to
    /// run. Only the T3 compatibility surface reaches this: its client can
    /// create a thread before anyone has typed into it. A task in this state is
    /// an ordinary open task with no messages — the absence of a sandbox
    /// lifecycle record already means "none was ever requested", which is why
    /// this needs no new task status.
    Skipped,
}

/// Result of a successful creation: the inserted row plus the provision result.
///
/// `task` is the in-memory row as inserted; DB-populated columns
/// (`created_at_db`, `updated_at`) are `None` because the row is not re-read
/// after insert. Callers needing those must re-fetch the task by id.
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
    #[error("{0}")]
    Preflight(String),
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
            TaskCreationError::Preflight(msg) => TaskError::Forbidden(msg),
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
    forks: Arc<dyn TaskForkDao>,
    repositories: Arc<dyn RepositoryReader>,
    /// Read by the `TaskOpener` port, which resolves a Loop's Workspace before
    /// it builds a [`NewTaskRequest`].
    pub(crate) workspaces: Arc<dyn WorkspaceResolver>,
    events: Option<Arc<dyn EventPublisher>>,
    starter: Arc<dyn SandboxStarter>,
    joiner: Arc<dyn SandboxJoiner>,
    preflight: Arc<dyn PreflightPort>,
}

impl TaskCreationService {
    // reason: independent injected collaborators (four DAOs, the event
    // publisher, the lifecycle seams, the preflight seam); bundling them behind
    // a one-use struct only shuffles the same fields.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        tasks: Arc<dyn TaskDao>,
        tags: Arc<dyn TagDao>,
        relations: Arc<dyn TaskRelationDao>,
        forks: Arc<dyn TaskForkDao>,
        repositories: Arc<dyn RepositoryReader>,
        workspaces: Arc<dyn WorkspaceResolver>,
        events: Option<Arc<dyn EventPublisher>>,
        starter: Arc<dyn SandboxStarter>,
        joiner: Arc<dyn SandboxJoiner>,
        preflight: Arc<dyn PreflightPort>,
    ) -> Self {
        Self {
            tasks,
            tags,
            relations,
            forks,
            repositories,
            workspaces,
            events,
            starter,
            joiner,
            preflight,
        }
    }

    /// Run the full creation sequence: short-id → preflight → insert → tags →
    /// relations → optional first message → provision → task event.
    ///
    /// `build_event_payload` produces the success-event payload from the
    /// inserted row (the HTTP path enriches it into `{ "task": … }`);
    /// `send_first_message` delivers `req.first_message` when present. Both run
    /// through the full `AppState` at the call site, which the service cannot
    /// hold.
    pub async fn create<EvFut, MsgFut>(
        &self,
        mut req: NewTaskRequest,
        build_event_payload: impl FnOnce(Task) -> EvFut + Send,
        send_first_message: impl FnOnce(String, FirstMessage) -> MsgFut + Send,
    ) -> Result<TaskCreationOutcome, TaskCreationError>
    where
        EvFut: Future<Output = serde_json::Value> + Send,
        MsgFut: Future<Output = ()> + Send,
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
        let persisted_checkout_mode = join_checkout_mode.or(req.checkout_mode);
        let short_id = self.generate_unique_short_id().await?;
        let now = chrono::Utc::now();
        let owner_user_id = req.created_by.clone();

        let mut row = Task {
            task_id: task_id.clone(),
            short_id: Some(short_id),
            created_at: now,
            last_message_at: now,
            agent_type: req.agent_type.clone(),
            repository_id: req.repository_id.clone(),
            status: TaskStatus::Open,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: req.template_id.clone(),
            vibe: req.vibe,
            name: req.name.clone(),
            description: req.description.clone(),
            site_id: None,
            site_provider: None,
            branch: req.branch.clone(),
            model: req.model.clone(),
            created_by: req.created_by.clone(),
            visibility: req.visibility,
            scope: Scope::User,
            owner_user_id,
            run_as_user_id: req.run_as_user_id.clone(),
            updated_at: None,
            append_system_prompt: req.append_system_prompt.clone(),
            headless: req.headless,
            ttl_minutes: req.ttl_minutes,
            inactive_ttl_days: req.inactive_ttl_days,
            loop_id: req.loop_id.clone(),
            workspace_id: Some(req.workspace_id.clone()),
            sandbox_id: req.sandbox_id.clone(),
            checkout_mode: persisted_checkout_mode,
            skills: req.skills.clone(),
        };

        // Repo is needed for preflight. Reuse a caller-supplied one if present
        // (the HTTP path already fetched it); otherwise look it up, which also
        // yields the 404 for an unknown id.
        let repo = if let Some(repo) = req.repository.take() {
            repo
        } else {
            // reason: non-HTTP creation paths (adapters, loops) resolve
            // the repository configured on the Loop, not user input; the
            // HTTP path passes an already access-checked repository.
            #[allow(clippy::disallowed_methods)]
            let sys_ctx = ScopeContext::system();
            self.repositories
                .describe(&sys_ctx, &req.repository_id)
                .await?
                .ok_or_else(|| TaskCreationError::RepoNotFound(req.repository_id.clone()))?
        };

        // Preflight. On failure persist the row as `Error` and publish a
        // sandbox error event (the `start_task` pattern) so the task surfaces as
        // errored rather than silently proceeding; then surface the typed error.
        if let Err(e) = self.preflight.validate(&row, &repo).await {
            tracing::warn!(task_id = %task_id, error = %e, "Task creation preflight failed");
            row.status = TaskStatus::Error;
            self.tasks.insert_task(&row).await?;
            self.publish_sandbox_error(&task_id, &e.user_message, &e.detail, "preflight_failed")
                .await;
            return Err(TaskCreationError::Preflight(e.user_message));
        }

        self.tasks.insert_task(&row).await?;
        // `insert_task` writes the Sandbox row for a request that named none,
        // under the task's own id. Carry it back onto the in-memory row: the
        // create response and the `task.created` event are built from this row,
        // not re-read.
        row.sandbox_id.get_or_insert_with(|| task_id.clone());
        tracing::info!(
            task_id = %task_id,
            created_by = ?row.created_by,
            repository_id = %row.repository_id,
            workspace_id = ?row.workspace_id,
            "Task created"
        );

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
            )
            .await?;
            if let Some(session) = &fork.session {
                self.forks
                    .insert(&TaskFork {
                        task_id: task_id.clone(),
                        source_task_id: fork.source_task_id.clone(),
                        at_turn: fork.at_turn,
                        source_session_id: session.source_session_id.clone(),
                        source_message_uuid: session.source_message_uuid.clone(),
                        source_agent_turn_id: session.source_agent_turn_id.clone(),
                    })
                    .await?;
            }
        }

        // Join an existing sandbox before delivering a first message. The DB
        // membership row must exist first so the sandbox-bound token can
        // authorize this task on later per-task RPCs.
        //
        // A join against a shared sandbox that has idle-stopped fails `NotReady`:
        // there is no pod to register the in-pod context on. That is not a
        // create failure — the membership `insert_task` already wrote is the
        // durable fact, and starting the shared sandbox below restores its
        // checkout and re-registers every member, this one included. So a
        // `NotReady` join defers to that start rather than 500-ing the create;
        // every other join error stays fatal and takes the inserted task back
        // down.
        let mut join_deferred_to_start = false;
        if let Some(checkout_mode) = join_checkout_mode {
            match self
                .joiner
                .join_task(&task_id, checkout_mode, &row.agent_type)
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
                    if let Err(cleanup_error) = self
                        .tasks
                        .delete_inserted_task_after_join_failure(&task_id)
                        .await
                    {
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

        // Optional first message (best-effort; the caller's callback logs).
        // Persist it before provisioning so the sandbox's authoritative startup
        // dispatch cannot race ahead of the initial prompt and find no pending
        // input.
        if let Some(first_message) = req.first_message {
            send_first_message(task_id.clone(), first_message).await;
        }

        // Provision (best-effort). Failure publishes a sandbox error event so
        // the task shows as errored; creation still succeeds.
        //
        // A task created with nothing to run gets no sandbox until something is
        // sent to it. Only the T3 compatibility surface asks for this; every
        // other caller leaves `provision` at its default and is unaffected.
        // Note this is not the same as "no first message": the bindings runtime
        // and the schedule runner create with `first_message: None` and deliver
        // it themselves, and they do need a sandbox.
        // A same-sandbox join that landed needs no start: the shared pod is
        // already running. One that deferred (`NotReady`) does — starting keys
        // off this task's `sandbox_id`, which is the shared one, so it wakes the
        // shared sandbox and its init registers this member. Both share the
        // start-or-error handling with a normal provision.
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

        // Task event.
        if let Some(events) = &self.events {
            let payload = build_event_payload(row.clone()).await;
            events
                .publish_task_event(
                    &task_id,
                    req.event_subtype,
                    Some(row.agent_type.as_str()),
                    payload,
                )
                .await;
        }

        Ok(TaskCreationOutcome {
            task: row,
            provision,
        })
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
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::tag::dao::MockTagDao;
    use crate::task::dao::MockTaskDao;
    use crate::task::relations::MockTaskRelationDao;
    use async_trait::async_trait;
    use moatless_core::events::test_support::RecordingPublisher;
    use moatless_ports::sandbox::{
        MockPreflightPort, MockSandboxJoiner, PreflightRejection, SandboxError,
    };
    use moatless_ports::workspace::{MockRepositoryReader, MockWorkspaceResolver};
    use moatless_types::repository::RepositoryKind;
    use moatless_types::task::CheckoutMode;
    use moatless_workspaces::repository::models::Repository;

    mockall::mock! {
        Starter {}
        #[async_trait]
        impl SandboxStarter for Starter {
            async fn start(&self, task_id: &str) -> Result<(), SandboxError>;
        }
    }

    const WORKSPACE: &str = "ws_repo1";

    fn sample_repo() -> Repository {
        Repository {
            id: "repo1".into(),
            name: "repo".into(),
            kind: RepositoryKind::Remote,
            remote_url: None,
            default_branch: None,
            description: None,
            provider: None,
            scope: Scope::User,
            owner_user_id: None,
            created_by: None,
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    fn sample_request() -> NewTaskRequest {
        NewTaskRequest {
            repository_id: "repo1".into(),
            agent_type: AgentType(AgentType::CLAUDE_CODE.into()),
            name: Some("t".into()),
            description: None,
            branch: None,
            model: None,
            created_by: Some("user1".into()),
            run_as_user_id: None,
            headless: false,
            visibility: Visibility::Private,
            loop_id: None,
            task_id: None,
            provision: true,
            workspace_id: WORKSPACE.into(),
            sandbox_id: None,
            checkout_mode: None,
            template_id: None,
            vibe: None,
            append_system_prompt: None,
            ttl_minutes: 15,
            inactive_ttl_days: 7,
            tag_ids: vec![],
            skills: vec![],
            parent: None,
            related_to: None,
            forked_from: None,
            first_message: None,
            event_subtype: TaskSubType::Created,
            repository: None,
        }
    }

    /// Preflight that always passes.
    fn preflight_ok() -> Arc<dyn PreflightPort> {
        let mut m = MockPreflightPort::new();
        m.expect_validate().returning(|_, _| Ok(()));
        Arc::new(m)
    }

    async fn empty_payload(_: Task) -> serde_json::Value {
        serde_json::json!({})
    }

    async fn no_message(_: String, _: FirstMessage) {}

    fn service(
        tasks: MockTaskDao,
        starter: MockStarter,
        joiner: MockSandboxJoiner,
        preflight: Arc<dyn PreflightPort>,
        publisher: Arc<RecordingPublisher>,
    ) -> TaskCreationService {
        let tags = MockTagDao::new();
        let relations = MockTaskRelationDao::new();
        let forks = crate::task::fork::MockTaskForkDao::new();
        let mut repos = MockRepositoryReader::new();
        repos
            .expect_describe()
            .returning(|_, _| Ok(Some(sample_repo())));
        TaskCreationService::new(
            Arc::new(tasks),
            Arc::new(tags),
            Arc::new(relations),
            Arc::new(forks),
            Arc::new(repos),
            Arc::new(MockWorkspaceResolver::new()),
            Some(publisher as Arc<dyn EventPublisher>),
            Arc::new(starter),
            Arc::new(joiner),
            preflight,
        )
    }

    fn joiner_never() -> MockSandboxJoiner {
        let mut joiner = MockSandboxJoiner::new();
        joiner.expect_join_task().never();
        joiner
    }

    #[tokio::test]
    async fn happy_path_inserts_open_row_and_publishes_created_event() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.status == TaskStatus::Open)
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter.expect_start().times(1).returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(
            tasks,
            starter,
            joiner_never(),
            preflight_ok(),
            publisher.clone(),
        );

        let outcome = svc
            .create(sample_request(), empty_payload, no_message)
            .await
            .expect("creation succeeds");

        assert_eq!(outcome.provision, ProvisionOutcome::Started);
        assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
        assert!(publisher.sandbox_statuses().is_empty());
    }

    /// The create response and the `task.created` event are built from this row
    /// rather than re-read, so a row left with `None` here publishes a task
    /// whose workspace no client can name.
    #[tokio::test]
    async fn the_persisted_and_returned_rows_carry_the_requested_workspace() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.workspace_id.as_deref() == Some(WORKSPACE))
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(
            tasks,
            starter,
            joiner_never(),
            preflight_ok(),
            publisher.clone(),
        );

        let outcome = svc
            .create(sample_request(), empty_payload, no_message)
            .await
            .expect("creation succeeds");

        assert_eq!(outcome.task.workspace_id.as_deref(), Some(WORKSPACE));
    }

    /// A global-scoped loop fires as its bot user. The task it spawns must
    /// still be `Scope::User` owned by that bot: the pod authenticates as
    /// `created_by`, and the gRPC control gate refuses a global or ownerless
    /// row, so anything else is a sandbox that can never fetch credentials.
    #[tokio::test]
    async fn a_task_is_always_user_scoped_and_owned_by_its_creator() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.scope == Scope::User && row.owner_user_id.as_deref() == Some("bot1"))
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);
        let mut req = sample_request();
        req.created_by = Some("bot1".into());

        let outcome = svc
            .create(req, empty_payload, no_message)
            .await
            .expect("creation succeeds");

        assert_eq!(outcome.task.scope, Scope::User);
        assert_eq!(outcome.task.owner_user_id.as_deref(), Some("bot1"));
    }

    #[tokio::test]
    async fn task_creation_persists_requested_model_on_the_task() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.model.as_deref() == Some("claude-opus-4-6"))
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);
        let mut req = sample_request();
        req.model = Some("claude-opus-4-6".to_string());

        let outcome = svc
            .create(req, empty_payload, no_message)
            .await
            .expect("creation succeeds");

        assert_eq!(outcome.task.model.as_deref(), Some("claude-opus-4-6"));
    }

    #[tokio::test]
    async fn first_message_is_sent_before_provisioning() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .times(1)
            .returning(|_| Ok(()));

        let mut starter = MockStarter::new();
        let start_order = Arc::clone(&order);
        starter.expect_start().times(1).returning(move |_| {
            start_order.lock().expect("order lock").push("start");
            Ok(())
        });

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);

        let mut req = sample_request();
        req.first_message = Some(FirstMessage {
            text: "hello".into(),
            model: None,
            agent_mode: None,
            effort: None,
            file_ids: vec![],
            contexts: vec![],
            sent_by: Some("user1".into()),
            message_uuid: None,
            skills: vec![],
        });

        let message_order = Arc::clone(&order);
        svc.create(req, empty_payload, move |_, _| {
            let message_order = Arc::clone(&message_order);
            async move {
                message_order.lock().expect("order lock").push("message");
            }
        })
        .await
        .expect("creation succeeds");

        assert_eq!(
            order.lock().expect("order lock").as_slice(),
            ["message", "start"]
        );
    }

    #[tokio::test]
    async fn preflight_failure_marks_errored_publishes_error_event_and_skips_provision() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.status == TaskStatus::Error)
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter.expect_start().never();

        let mut preflight = MockPreflightPort::new();
        preflight.expect_validate().returning(|_, _| {
            Err(PreflightRejection {
                user_message: "GitHub access is not configured for this repository scope.".into(),
                detail: "GitHub account not connected for user u".into(),
            })
        });

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(
            tasks,
            starter,
            joiner_never(),
            Arc::new(preflight),
            publisher.clone(),
        );

        let err = svc
            .create(sample_request(), empty_payload, no_message)
            .await
            .expect_err("preflight failure surfaces");

        assert!(matches!(err, TaskCreationError::Preflight(_)));
        assert_eq!(publisher.sandbox_statuses(), vec![SandboxStatus::Error]);
        assert!(publisher.task_subtypes().is_empty());
    }

    #[tokio::test]
    async fn provision_failure_still_succeeds_and_publishes_sandbox_error() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.status == TaskStatus::Open)
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter
            .expect_start()
            .times(1)
            .returning(|_| Err(SandboxError::Other("boom".into())));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(
            tasks,
            starter,
            joiner_never(),
            preflight_ok(),
            publisher.clone(),
        );

        let outcome = svc
            .create(sample_request(), empty_payload, no_message)
            .await
            .expect("creation still succeeds");

        assert_eq!(outcome.provision, ProvisionOutcome::Failed);
        assert_eq!(publisher.sandbox_statuses(), vec![SandboxStatus::Error]);
        assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
    }

    #[tokio::test]
    async fn short_id_retries_on_collision() {
        let mut tasks = MockTaskDao::new();
        let mut seq = mockall::Sequence::new();
        tasks
            .expect_exists_by_short_id()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| Ok(true));
        tasks
            .expect_exists_by_short_id()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(
            tasks,
            starter,
            joiner_never(),
            preflight_ok(),
            publisher.clone(),
        );

        let outcome = svc
            .create(sample_request(), empty_payload, no_message)
            .await
            .expect("creation succeeds after retry");

        assert_eq!(outcome.provision, ProvisionOutcome::Started);
    }

    /// T3 mints the Thread id, navigates to it, and keys its own state on it.
    /// Minting our own here would leave the client watching a Thread that never
    /// arrives.
    #[tokio::test]
    async fn a_supplied_task_id_is_used_verbatim() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.task_id == "3f1c9d42-0a7e-4b19-8c55-11d2ab6e7f30")
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(
            tasks,
            starter,
            joiner_never(),
            preflight_ok(),
            publisher.clone(),
        );

        let mut req = sample_request();
        req.task_id = Some("3f1c9d42-0a7e-4b19-8c55-11d2ab6e7f30".into());
        let outcome = svc
            .create(req, empty_payload, no_message)
            .await
            .expect("creation succeeds");

        assert_eq!(outcome.task.task_id, "3f1c9d42-0a7e-4b19-8c55-11d2ab6e7f30");
    }

    /// A Thread created without a first message is a Task nobody has spoken to
    /// yet. Provisioning it would burn a Sandbox on a Thread that may never be
    /// used.
    #[tokio::test]
    async fn provisioning_is_skipped_when_the_request_does_not_ask_for_it() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockStarter::new();
        starter.expect_start().never();

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(
            tasks,
            starter,
            joiner_never(),
            preflight_ok(),
            publisher.clone(),
        );

        let mut req = sample_request();
        req.provision = false;
        let outcome = svc
            .create(req, empty_payload, no_message)
            .await
            .expect("creation succeeds");

        assert_eq!(outcome.provision, ProvisionOutcome::Skipped);
        assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
        assert!(publisher.sandbox_statuses().is_empty());
    }

    #[tokio::test]
    async fn shared_sandbox_create_joins_before_first_message_and_skips_provisioning() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| {
                row.sandbox_id.as_deref() == Some("sandbox-1")
                    && row.checkout_mode == Some(CheckoutMode::Shared)
            })
            .times(1)
            .returning(|_| Ok(()));

        let mut starter = MockStarter::new();
        starter.expect_start().never();
        let mut joiner = MockSandboxJoiner::new();
        let join_order = Arc::clone(&order);
        joiner
            .expect_join_task()
            .withf(|task_id, checkout_mode, agent_type| {
                !task_id.is_empty()
                    && *checkout_mode == CheckoutMode::Shared
                    // The joining Task's own harness, not the shared pod's.
                    && agent_type.as_str() == AgentType::CLAUDE_CODE
            })
            .times(1)
            .returning(move |_, _, _| {
                join_order.lock().expect("order lock").push("join");
                Ok(())
            });
        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, joiner, preflight_ok(), publisher.clone());

        let mut req = sample_request();
        req.sandbox_id = Some("sandbox-1".to_string());
        req.first_message = Some(FirstMessage {
            text: "hello".into(),
            model: None,
            agent_mode: None,
            effort: None,
            file_ids: vec![],
            contexts: vec![],
            sent_by: Some("user1".into()),
            message_uuid: None,
            skills: vec![],
        });

        let message_order = Arc::clone(&order);
        let outcome = svc
            .create(req, empty_payload, move |_, _| {
                let message_order = Arc::clone(&message_order);
                async move {
                    message_order.lock().expect("order lock").push("message");
                }
            })
            .await
            .expect("shared sandbox creation succeeds");

        assert_eq!(outcome.provision, ProvisionOutcome::Started);
        assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
        assert_eq!(
            order.lock().expect("order lock").as_slice(),
            ["join", "message"]
        );
    }

    #[tokio::test]
    async fn shared_sandbox_join_failure_cleans_up_before_first_message() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .times(1)
            .returning(|_| Ok(()));
        let cleanup_order = Arc::clone(&order);
        tasks
            .expect_delete_inserted_task_after_join_failure()
            .withf(|task_id| !task_id.is_empty())
            .times(1)
            .returning(move |_| {
                cleanup_order.lock().expect("order lock").push("cleanup");
                Ok(true)
            });

        let mut starter = MockStarter::new();
        starter.expect_start().never();
        let mut joiner = MockSandboxJoiner::new();
        let join_order = Arc::clone(&order);
        joiner.expect_join_task().times(1).returning(move |_, _, _| {
            join_order.lock().expect("order lock").push("join");
            // A genuine join failure — not `NotReady`, which defers to a start
            // (covered by `shared_sandbox_join_not_ready_starts_the_shared_sandbox`).
            Err(SandboxError::GrpcClient("connection refused".to_string()))
        });
        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, joiner, preflight_ok(), publisher);

        let mut req = sample_request();
        req.sandbox_id = Some("sandbox-1".to_string());
        req.first_message = Some(FirstMessage {
            text: "hello".into(),
            model: None,
            agent_mode: None,
            effort: None,
            file_ids: vec![],
            contexts: vec![],
            sent_by: Some("user1".into()),
            message_uuid: None,
            skills: vec![],
        });

        let err = svc
            .create(req, empty_payload, |_, _| async {
                panic!("first message must not send after join failure");
            })
            .await
            .expect_err("join failure rejects create");

        assert!(matches!(err, TaskCreationError::SandboxJoin(_)));
        assert_eq!(
            order.lock().expect("order lock").as_slice(),
            ["join", "cleanup"]
        );
    }

    #[tokio::test]
    async fn shared_sandbox_join_not_ready_starts_the_shared_sandbox() {
        // A same-sandbox create whose target has idle-stopped: the join reports
        // `NotReady` (no pod to register on), which must not fail the create. The
        // membership row is already durable, so creation starts the shared
        // sandbox — waking it — and still sends the first message.
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .times(1)
            .returning(|_| Ok(()));
        // A deferred join never cleans up the inserted task.
        tasks
            .expect_delete_inserted_task_after_join_failure()
            .never();

        let mut starter = MockStarter::new();
        let start_order = Arc::clone(&order);
        starter.expect_start().times(1).returning(move |_| {
            start_order.lock().expect("order lock").push("start");
            Ok(())
        });
        let mut joiner = MockSandboxJoiner::new();
        let join_order = Arc::clone(&order);
        joiner.expect_join_task().times(1).returning(move |_, _, _| {
            join_order.lock().expect("order lock").push("join");
            Err(SandboxError::NotReady("Sandbox is stopped".to_string()))
        });
        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, joiner, preflight_ok(), publisher.clone());

        let mut req = sample_request();
        req.sandbox_id = Some("sandbox-1".to_string());
        req.first_message = Some(FirstMessage {
            text: "hello".into(),
            model: None,
            agent_mode: None,
            effort: None,
            file_ids: vec![],
            contexts: vec![],
            sent_by: Some("user1".into()),
            message_uuid: None,
            skills: vec![],
        });

        let message_order = Arc::clone(&order);
        let outcome = svc
            .create(req, empty_payload, move |_, _| {
                let message_order = Arc::clone(&message_order);
                async move {
                    message_order.lock().expect("order lock").push("message");
                }
            })
            .await
            .expect("a not-ready shared sandbox is started, not rejected");

        assert_eq!(outcome.provision, ProvisionOutcome::Started);
        // The stopped shared sandbox produces no error event: it is being woken,
        // not failing.
        assert!(publisher.sandbox_statuses().is_empty());
        assert_eq!(
            order.lock().expect("order lock").as_slice(),
            ["join", "message", "start"]
        );
    }

    #[tokio::test]
    async fn checkout_mode_without_sandbox_id_is_rejected_before_persistence() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().never();
        tasks.expect_insert_task().never();

        let mut starter = MockStarter::new();
        starter.expect_start().never();
        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);

        let mut req = sample_request();
        req.checkout_mode = Some(CheckoutMode::Shared);
        let err = svc
            .create(req, empty_payload, no_message)
            .await
            .expect_err("checkout mode without sandbox is rejected");

        assert!(matches!(
            err,
            TaskCreationError::BadRequest(msg) if msg.contains("checkout_mode requires sandbox_id")
        ));
    }
}
