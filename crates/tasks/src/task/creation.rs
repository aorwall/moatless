//! Task creation service — the single owner of the create-then-provision
//! sequence.
//!
//! Creating a task means: generate a unique short id, run the GitHub-token
//! preflight, insert the task row, assign tags, record relations, send an
//! optional first message, provision the sandbox, and publish a task event. This
//! sequence used to be copy-pasted across the HTTP handler and three
//! loop/adapter paths; [`TaskCreationService`] collapses it behind one entry
//! point so a provisioning fix lands once instead of four times.
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
//!   silently sitting dead, while creation itself still succeeds.

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;

use moatless_backend_core::sandbox::SandboxStarter;
use moatless_backend_core::scope::{Scope, ScopeContext};
use moatless_events::{SandboxEventPayload, SandboxStatus, TaskSubType};

use moatless_core::events::EventPublisher;
use moatless_workspaces::repository::dao::RepositoryDao;
use moatless_workspaces::repository::models::Repository;
use moatless_core::shared_models::{AgentMode, AgentType};
use moatless_core::tag::dao::TagDao;
use crate::task::dao::TaskDao;
use crate::task::error::TaskError;
use crate::task::models::{Task, TaskStatus, Visibility};
use crate::task::relations::{
    record_created_by, record_related_to, CreatedByParent, RelatedTask, TaskRelationDao,
};
use crate::task::schemas::MessageContextInput;

/// Preflight seam: GitHub-token validation before a sandbox is provisioned.
/// Wired once in state assembly (`RealPreflight`, in `sandbox::preflight`) so
/// every creation path gets the same check for free; mocked in unit tests.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PreflightPort: Send + Sync {
    async fn validate(&self, task: &Task, repo: &Repository) -> Result<(), PreflightRejection>;
}

/// Band-clean preflight failure returned by [`PreflightPort`]. The
/// sandbox-control implementation pre-computes both strings from its rich
/// `PreflightError` so the tasks band names no preflight-credential type:
/// `user_message` drives the 403 body + `sandbox.error` event, `detail` is the
/// underlying failure's `Display` for the log + event detail field.
#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct PreflightRejection {
    pub user_message: String,
    pub detail: String,
}

/// Normalised task-creation input. Every call path (HTTP handler, loops,
/// adapters) translates its domain input into this; the service never inspects
/// HTTP payloads, webhooks, or loop configs.
pub struct NewTaskRequest {
    pub repository_id: String,
    pub agent_type: AgentType,
    pub name: Option<String>,
    pub description: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub created_by: Option<String>,
    pub headless: bool,
    pub visibility: Visibility,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub loop_id: Option<String>,
    pub workspace_id: Option<String>,
    pub template_id: Option<String>,
    pub vibe: Option<bool>,
    pub append_system_prompt: Option<String>,
    pub ttl_minutes: i32,
    pub inactive_ttl_days: i32,
    /// Pre-resolved tag ids to attach (adapters pass their Loop's tag ids;
    /// the HTTP path passes none).
    pub tag_ids: Vec<String>,
    /// Optional `created_by` parent relation to record after insert.
    pub parent: Option<CreatedByParent>,
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
    Preflight(String),
    #[error("failed to generate a unique short id")]
    ShortId,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<TaskCreationError> for TaskError {
    fn from(e: TaskCreationError) -> Self {
        match e {
            TaskCreationError::RepoNotFound(id) => TaskError::RepoNotFound(id),
            TaskCreationError::Preflight(msg) => TaskError::Forbidden(msg),
            TaskCreationError::ShortId => {
                TaskError::Internal("Failed to generate unique short ID".to_string())
            }
            TaskCreationError::Db(e) => TaskError::Db(e),
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
    repositories: Arc<dyn RepositoryDao>,
    events: Option<Arc<dyn EventPublisher>>,
    starter: Arc<dyn SandboxStarter>,
    preflight: Arc<dyn PreflightPort>,
}

impl TaskCreationService {
    // reason: independent injected collaborators (four DAOs, the event
    // publisher, the provision seam, the preflight seam); bundling them behind a
    // one-use struct only shuffles the same fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tasks: Arc<dyn TaskDao>,
        tags: Arc<dyn TagDao>,
        relations: Arc<dyn TaskRelationDao>,
        repositories: Arc<dyn RepositoryDao>,
        events: Option<Arc<dyn EventPublisher>>,
        starter: Arc<dyn SandboxStarter>,
        preflight: Arc<dyn PreflightPort>,
    ) -> Self {
        Self {
            tasks,
            tags,
            relations,
            repositories,
            events,
            starter,
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
        let short_id = self.generate_unique_short_id().await?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let mut row = Task {
            task_id: task_id.clone(),
            short_id: Some(short_id),
            created_at: now.clone(),
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
            scope: req.scope,
            owner_user_id: req.owner_user_id.clone(),
            created_at_db: None,
            updated_at: None,
            append_system_prompt: req.append_system_prompt.clone(),
            headless: req.headless,
            ttl_minutes: req.ttl_minutes,
            inactive_ttl_days: req.inactive_ttl_days,
            loop_id: req.loop_id.clone(),
            workspace_id: req.workspace_id.clone(),
            disabled_servers: None,
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
                .find_by_id(&req.repository_id, &sys_ctx)
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

        // `insert_task` resolves the repository's shadow workspace when the
        // caller supplied none, so the persisted row always has one. Carry it
        // back onto the in-memory row: the create response and the
        // `task.created` event are built from this row, not re-read, and both
        // label the workspace the task runs in.
        let workspace_id = self.tasks.insert_task(&row).await?;
        row.workspace_id = Some(workspace_id);
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
        let provision = if req.provision {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use moatless_workspaces::repository::dao::MockRepositoryDao;
    use moatless_workspaces::repository::models::{Repository, RepositoryKind};
    use moatless_core::tag::dao::MockTagDao;
    use crate::task::dao::MockTaskDao;
    use crate::task::relations::MockTaskRelationDao;
    use moatless_core::events::test_support::RecordingPublisher;
    use moatless_backend_core::sandbox::SandboxError;

    mockall::mock! {
        Starter {}
        #[async_trait]
        impl SandboxStarter for Starter {
            async fn start(&self, task_id: &str) -> Result<(), SandboxError>;
        }
    }

    /// What `insert_task` resolves for a request that names no workspace: the
    /// repository's shadow workspace.
    const SHADOW_WORKSPACE: &str = "ws_repo1";

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
            access_status: "unchecked".into(),
            access_checked_at: None,
            access_error: None,
            deleted: None,
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
            headless: false,
            visibility: Visibility::Private,
            scope: Scope::User,
            owner_user_id: Some("user1".into()),
            loop_id: None,
            task_id: None,
            provision: true,
            workspace_id: None,
            template_id: None,
            vibe: None,
            append_system_prompt: None,
            ttl_minutes: 15,
            inactive_ttl_days: 7,
            tag_ids: vec![],
            parent: None,
            related_to: None,
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
        preflight: Arc<dyn PreflightPort>,
        publisher: Arc<RecordingPublisher>,
    ) -> TaskCreationService {
        let tags = MockTagDao::new();
        let relations = MockTaskRelationDao::new();
        let mut repos = MockRepositoryDao::new();
        repos
            .expect_find_by_id()
            .returning(|_, _| Ok(Some(sample_repo())));
        TaskCreationService::new(
            Arc::new(tasks),
            Arc::new(tags),
            Arc::new(relations),
            Arc::new(repos),
            Some(publisher as Arc<dyn EventPublisher>),
            Arc::new(starter),
            preflight,
        )
    }

    #[tokio::test]
    async fn happy_path_inserts_open_row_and_publishes_created_event() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.status == TaskStatus::Open)
            .times(1)
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));
        let mut starter = MockStarter::new();
        starter.expect_start().times(1).returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher.clone());

        let outcome = svc
            .create(sample_request(), empty_payload, no_message)
            .await
            .expect("creation succeeds");

        assert_eq!(outcome.provision, ProvisionOutcome::Started);
        assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
        assert!(publisher.sandbox_statuses().is_empty());
    }

    /// The request named no workspace, so the insert resolved the repository's
    /// shadow one. The create response and the `task.created` event are built
    /// from this row rather than re-read, so a row left with `None` here
    /// publishes a task whose workspace no client can name.
    #[tokio::test]
    async fn the_returned_row_carries_the_workspace_the_insert_resolved() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.workspace_id.is_none())
            .times(1)
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher.clone());

        let outcome = svc
            .create(sample_request(), empty_payload, no_message)
            .await
            .expect("creation succeeds");

        assert_eq!(outcome.task.workspace_id.as_deref(), Some(SHADOW_WORKSPACE));
    }

    #[tokio::test]
    async fn task_creation_persists_requested_model_on_the_task() {
        let mut tasks = MockTaskDao::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.model.as_deref() == Some("claude-opus-4-6"))
            .times(1)
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher);
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
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));

        let mut starter = MockStarter::new();
        let start_order = Arc::clone(&order);
        starter.expect_start().times(1).returning(move |_| {
            start_order.lock().expect("order lock").push("start");
            Ok(())
        });

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher);

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
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));
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
        let svc = service(tasks, starter, Arc::new(preflight), publisher.clone());

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
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));
        let mut starter = MockStarter::new();
        starter
            .expect_start()
            .times(1)
            .returning(|_| Err(SandboxError::Other("boom".into())));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher.clone());

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
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher.clone());

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
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));
        let mut starter = MockStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher.clone());

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
            .returning(|_| Ok(SHADOW_WORKSPACE.to_string()));
        let mut starter = MockStarter::new();
        starter.expect_start().never();

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher.clone());

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
}
