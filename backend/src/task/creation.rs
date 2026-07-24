//! Task creation service — the single owner of the create-then-provision
//! sequence.
//!
//! Creating a task means: generate a unique short id, run the GitHub-token
//! preflight, insert the task row, assign tags, record relations, provision the
//! sandbox, send an optional first message, and publish a task event. This
//! sequence used to be copy-pasted across the HTTP handler and three
//! trigger/adapter paths; [`TaskCreationService`] collapses it behind one entry
//! point so a provisioning fix lands once instead of four times.
//!
//! Callers normalise their domain input (HTTP payload, webhook, trigger config)
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

use moatless_events::{SandboxEventPayload, SandboxStatus, TaskSubType};
use moatless_backend_core::scope::Scope;

use crate::events::EventPublisher;
use crate::repository::dao::RepoRepository;
use crate::repository::models::Repository;
use crate::sandbox::preflight::{
    preflight_validate_github_token, GithubValidator, PreflightCredentials, PreflightError,
};
use crate::shared_models::{AgentMode, AgentType};
use crate::tag::dao::TagRepository;
use crate::task::dao::TaskRepository;
use crate::task::error::TaskError;
use crate::task::models::{Task, TaskStatus, Visibility};
use crate::task::relations::{
    record_created_by, record_related_to, CreatedByParent, RelatedTask, TaskRelationRepository,
};
use crate::task::schemas::MessageContextInput;

/// Provision seam: the one lifecycle capability the creation sequence needs.
/// Implemented by the concrete `Lifecycle` handle in production; mocked in unit
/// tests so provisioning can be exercised without a database.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SandboxStarter: Send + Sync {
    /// Best-effort provision. `Ok(())` means a start was initiated; `Err`
    /// carries a human-readable reason for the sandbox error event.
    async fn start(&self, task_id: &str) -> Result<(), String>;
}

#[async_trait]
impl SandboxStarter for crate::sandbox::lifecycle::Lifecycle {
    async fn start(&self, task_id: &str) -> Result<(), String> {
        crate::sandbox::lifecycle::Lifecycle::start(self, task_id)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Preflight seam: GitHub-token validation before a sandbox is provisioned.
/// Wired once in state assembly ([`RealPreflight`]) so every creation path gets
/// the same check for free; mocked in unit tests.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PreflightPort: Send + Sync {
    async fn validate(&self, task: &Task, repo: &Repository) -> Result<(), PreflightError>;
}

/// Production preflight — holds exactly the credential handles
/// `preflight_validate_github_token` consumes plus the network validator seam.
pub struct RealPreflight {
    creds: PreflightCredentials,
    validator: Arc<dyn GithubValidator>,
}

impl RealPreflight {
    pub fn new(creds: PreflightCredentials, validator: Arc<dyn GithubValidator>) -> Self {
        Self { creds, validator }
    }
}

#[async_trait]
impl PreflightPort for RealPreflight {
    async fn validate(&self, task: &Task, repo: &Repository) -> Result<(), PreflightError> {
        preflight_validate_github_token(&self.creds, task, repo, self.validator.as_ref()).await
    }
}

/// Normalised task-creation input. Every call path (HTTP handler, triggers,
/// adapters) translates its domain input into this; the service never inspects
/// HTTP payloads, webhooks, or trigger configs.
pub struct NewTaskRequest {
    pub repository_id: String,
    pub agent_type: AgentType,
    pub name: Option<String>,
    pub description: Option<String>,
    pub branch: Option<String>,
    pub created_by: Option<String>,
    pub headless: bool,
    pub visibility: Visibility,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub trigger_id: Option<String>,
    pub workspace_id: Option<String>,
    pub template_id: Option<String>,
    pub vibe: Option<bool>,
    pub append_system_prompt: Option<String>,
    pub ttl_minutes: i32,
    pub inactive_ttl_days: i32,
    /// Pre-resolved tag ids to attach (adapters pass their trigger's tag ids;
    /// the HTTP path passes none).
    pub tag_ids: Vec<String>,
    /// Optional `created_by` parent relation to record after insert.
    pub parent: Option<CreatedByParent>,
    /// Optional `related_to` peer relation to record after insert.
    pub related_to: Option<RelatedTask>,
    /// Optional first message; when present the service invokes the caller's
    /// send callback after provisioning.
    pub first_message: Option<FirstMessage>,
    /// Task-event subtype to publish — `Created` on the HTTP path, `Open` on
    /// trigger/adapter paths. Kept per-caller by design.
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
    pub file_ids: Vec<String>,
    pub contexts: Vec<MessageContextInput>,
    pub sent_by: Option<String>,
}

/// Whether the best-effort provision step started the sandbox. Ticket 02's
/// `schedule_runner` records a `provision_failed` metric off this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionOutcome {
    Started,
    Failed,
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
    tasks: Arc<dyn TaskRepository>,
    tags: Arc<dyn TagRepository>,
    relations: Arc<dyn TaskRelationRepository>,
    repositories: Arc<dyn RepoRepository>,
    events: Option<Arc<dyn EventPublisher>>,
    starter: Arc<dyn SandboxStarter>,
    preflight: Arc<dyn PreflightPort>,
}

impl TaskCreationService {
    // reason: independent injected collaborators (four repositories, the event
    // publisher, the provision seam, the preflight seam); bundling them behind a
    // one-use struct only shuffles the same fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tasks: Arc<dyn TaskRepository>,
        tags: Arc<dyn TagRepository>,
        relations: Arc<dyn TaskRelationRepository>,
        repositories: Arc<dyn RepoRepository>,
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
    /// relations → provision → optional first message → task event.
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
        let task_id = uuid::Uuid::new_v4().to_string();
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
            created_by: req.created_by.clone(),
            visibility: req.visibility,
            scope: req.scope,
            team_id: req.team_id.clone(),
            owner_user_id: req.owner_user_id.clone(),
            created_at_db: None,
            updated_at: None,
            append_system_prompt: req.append_system_prompt.clone(),
            headless: req.headless,
            ttl_minutes: req.ttl_minutes,
            inactive_ttl_days: req.inactive_ttl_days,
            trigger_id: req.trigger_id.clone(),
            workspace_id: req.workspace_id.clone(),
            disabled_servers: None,
        };

        // Repo is needed for preflight. Reuse a caller-supplied one if present
        // (the HTTP path already fetched it); otherwise look it up, which also
        // yields the 404 for an unknown id.
        let repo = match req.repository.take() {
            Some(repo) => repo,
            None => self
                .repositories
                .find_repository_by_id(&req.repository_id)
                .await?
                .ok_or_else(|| TaskCreationError::RepoNotFound(req.repository_id.clone()))?,
        };

        // Preflight. On failure persist the row as `Error` and publish a
        // sandbox error event (the `start_task` pattern) so the task surfaces as
        // errored rather than silently proceeding; then surface the typed error.
        if let Err(e) = self.preflight.validate(&row, &repo).await {
            let msg = e.user_message();
            tracing::warn!(task_id = %task_id, error = %e, "Task creation preflight failed");
            row.status = TaskStatus::Error;
            self.tasks.insert_task(&row).await?;
            self.publish_sandbox_error(&task_id, &msg, &e.to_string(), "preflight_failed")
                .await;
            return Err(TaskCreationError::Preflight(msg));
        }

        self.tasks.insert_task(&row).await?;
        tracing::info!(
            task_id = %task_id,
            created_by = ?row.created_by,
            repository_id = %row.repository_id,
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

        // Provision (best-effort). Failure publishes a sandbox error event so
        // the task shows as errored; creation still succeeds.
        let provision = match self.starter.start(&task_id).await {
            Ok(()) => ProvisionOutcome::Started,
            Err(e) => {
                tracing::warn!(task_id = %task_id, error = %e, "Sandbox start failed during task creation");
                self.publish_sandbox_error(
                    &task_id,
                    "Sandbox provisioning failed during task creation.",
                    &e,
                    "provision_failed",
                )
                .await;
                ProvisionOutcome::Failed
            }
        };

        // Optional first message (best-effort; the caller's callback logs).
        if let Some(first_message) = req.first_message {
            send_first_message(task_id.clone(), first_message).await;
        }

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

    use crate::repository::dao::MockRepoRepository;
    use crate::repository::models::{Repository, RepositoryKind};
    use crate::tag::dao::MockTagRepository;
    use crate::task::dao::MockTaskRepository;
    use crate::task::relations::MockTaskRelationRepository;
    use crate::test_support::RecordingPublisher;

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
            team_id: None,
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
            created_by: Some("user1".into()),
            headless: false,
            visibility: Visibility::Private,
            scope: Scope::User,
            team_id: None,
            owner_user_id: Some("user1".into()),
            trigger_id: None,
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
        tasks: MockTaskRepository,
        starter: MockSandboxStarter,
        preflight: Arc<dyn PreflightPort>,
        publisher: Arc<RecordingPublisher>,
    ) -> TaskCreationService {
        let tags = MockTagRepository::new();
        let relations = MockTaskRelationRepository::new();
        let mut repos = MockRepoRepository::new();
        repos
            .expect_find_repository_by_id()
            .returning(|_| Ok(Some(sample_repo())));
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
        let mut tasks = MockTaskRepository::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.status == TaskStatus::Open)
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockSandboxStarter::new();
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

    #[tokio::test]
    async fn preflight_failure_marks_errored_publishes_error_event_and_skips_provision() {
        let mut tasks = MockTaskRepository::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.status == TaskStatus::Error)
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockSandboxStarter::new();
        starter.expect_start().never();

        let mut preflight = MockPreflightPort::new();
        preflight.expect_validate().returning(|_, _| {
            Err(PreflightError::NoGithubToken {
                user_id: "u".into(),
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
        let mut tasks = MockTaskRepository::new();
        tasks.expect_exists_by_short_id().returning(|_| Ok(false));
        tasks
            .expect_insert_task()
            .withf(|row| row.status == TaskStatus::Open)
            .times(1)
            .returning(|_| Ok(()));
        let mut starter = MockSandboxStarter::new();
        starter
            .expect_start()
            .times(1)
            .returning(|_| Err("boom".into()));

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
        let mut tasks = MockTaskRepository::new();
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
        tasks.expect_insert_task().times(1).returning(|_| Ok(()));
        let mut starter = MockSandboxStarter::new();
        starter.expect_start().returning(|_| Ok(()));

        let publisher = Arc::new(RecordingPublisher::new());
        let svc = service(tasks, starter, preflight_ok(), publisher.clone());

        let outcome = svc
            .create(sample_request(), empty_payload, no_message)
            .await
            .expect("creation succeeds after retry");

        assert_eq!(outcome.provision, ProvisionOutcome::Started);
    }
}
