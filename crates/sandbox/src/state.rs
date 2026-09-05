//! Shared application state for the sandbox service.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use moatless_types::workspace::sanitize_mount_name;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::agent_state::AgentState;
use crate::agent::handle::{AgentHandle, spawn_agent};
use crate::agent::heartbeat::Heartbeat;
use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
use crate::backend_client::BackendClient;
use crate::commands::CommandRegistry;
use crate::config::Config;
use crate::credentials::Dispatcher;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::events::GrpcEventPublisher;
use crate::init::InitPhase;
use crate::init::codex_watcher::WatcherHandle;
use crate::init::workspace_sync::WorkspaceSyncTracker;
use crate::supervisor::SupervisorHealth;
use crate::terminal::TerminalManager;

/// Per-task runtime state inside a sandbox process.
///
/// A warm sandbox starts with one context keyed by the boot task. Joined tasks
/// get their own context so agent status, turns, prompts, scheduled jobs, and
/// dispatch single-flight state stay isolated by task.
#[derive(Clone)]
pub struct TaskContext {
    /// Task this context serves.
    pub task_id: String,
    /// Workspace root used by this task.
    pub workspace_dir: String,
    /// Primary checkout used by this task.
    pub checkout_dir: String,
    /// Checkout relationship requested by the backend for this member.
    pub checkout_mode: CheckoutMode,
    /// Agent execution state (status, session, cancel token, pending prompts).
    pub agent: Arc<AgentState>,
    /// Agent command handle — send messages via `agent_handle.send()`.
    pub agent_handle: AgentHandle,
    /// Registry of deferred-work jobs (Monitor crons + `ScheduleWakeup` timers)
    /// spawned by the permission handler / agent lifecycle service.
    pub scheduled_jobs: Arc<ScheduledJobRegistry>,
    /// Commands this task started through `moat cmd`. The only source of
    /// `busy_processes`, so a process started any other way cannot defer an
    /// idle stop.
    pub commands: Arc<CommandRegistry>,
    /// Single-flight guard for this task's pending-message dispatch.
    pub dispatch_lock: Arc<Mutex<()>>,
    /// Tracks credential failures observed on this task's agent stderr.
    pub credential_failures: CredentialFailureDetector,
    /// Last system prompt fetched for this task. Used as a fallback when
    /// `BackendClient::get_system_prompt` fails on a subsequent run.
    pub system_prompt_cache: Arc<RwLock<Option<String>>>,
}

impl TaskContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: String,
        workspace_dir: String,
        checkout_dir: String,
        checkout_mode: CheckoutMode,
        agent: Arc<AgentState>,
        agent_handle: AgentHandle,
        scheduled_jobs: Arc<ScheduledJobRegistry>,
        commands: Arc<CommandRegistry>,
        credential_failures: CredentialFailureDetector,
        system_prompt_cache: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            task_id,
            workspace_dir,
            checkout_dir,
            checkout_mode,
            agent,
            agent_handle,
            scheduled_jobs,
            commands,
            dispatch_lock: Default::default(),
            credential_failures,
            system_prompt_cache,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutMode {
    Shared,
    Isolated,
}

impl CheckoutMode {
    fn parse(raw: Option<&str>) -> Result<Self, RegisterTaskContextError> {
        match raw.unwrap_or("shared") {
            "shared" | "" => Ok(Self::Shared),
            "isolated" => Ok(Self::Isolated),
            other => Err(RegisterTaskContextError::InvalidCheckoutMode(
                other.to_string(),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Isolated => "isolated",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TaskContextLookupError {
    #[error("task is not registered in this sandbox process: {0}")]
    NotFound(String),
    #[error("task id is required when multiple tasks are registered")]
    Ambiguous,
    #[error("sandbox has no registered task contexts")]
    Empty,
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterTaskContextError {
    #[error("task_id is required")]
    EmptyTaskId,
    #[error("unsupported checkout_mode: {0}")]
    InvalidCheckoutMode(String),
    #[error("failed to materialize isolated checkout: {0}")]
    CheckoutMaterialization(String),
}

/// Shared state passed to all axum handlers via `State` extractor.
///
/// All fields are cheap to clone (Arc-wrapped or internally Arc'd).
#[derive(Clone)]
pub struct SandboxState {
    /// Immutable configuration.
    pub config: Arc<Config>,
    /// gRPC event publisher.
    pub events: GrpcEventPublisher,
    /// Per-task runtime contexts registered in this sandbox process.
    pub task_contexts: Arc<RwLock<HashMap<String, Arc<TaskContext>>>>,
    /// Serializes task-context materialization so idempotent `JoinTask` replays
    /// cannot race the same isolated checkout directory.
    pub task_context_registration: Arc<Mutex<()>>,
    /// Liveness heartbeat bumped by the agent lifecycle task. Read by the
    /// `/livez` probe to detect a wedged service (see [`crate::agent::heartbeat`]).
    pub heartbeat: Heartbeat,
    /// Backend client for gRPC communication (message polling, acknowledgment).
    pub backend_client: Arc<dyn BackendClient>,
    /// Current initialization phase — tracked for health probes and consumer gating.
    pub init_phase: Arc<RwLock<InitPhase>>,
    /// Credentials fetched during init, passed to agent subprocess via `ClaudeOptions.env`.
    ///
    /// Git-provider tokens (`GITHUB_TOKEN`, `GH_TOKEN`, `GIT_TOKEN`) are intentionally
    /// stripped from this map before it reaches the agent subprocess — git/gh operations
    /// inside the agent read tokens fresh from disk (`~/.config/gh/hosts.yml`) so inherited
    /// env would only risk staleness across long-running turns.
    pub credentials: Arc<RwLock<HashMap<String, String>>>,
    /// Cancellation token tied to the sandbox lifecycle.
    ///
    /// Used by background tasks (e.g. the credential refresher) to shut down
    /// cleanly when the pod is stopping.
    pub lifecycle_cancel: CancellationToken,
    /// Generic dispatcher for backend-pushed credential refreshes.
    ///
    /// The gRPC server hands the rotated value to this dispatcher, which
    /// routes to the appropriate sink (file write, env var update, etc.).
    pub credential_dispatcher: Arc<Dispatcher>,
    /// Handle to the Codex auth watcher, used by [`crate::credentials::sinks`]
    /// to seed the watcher cache after a backend-driven write so the resulting
    /// filesystem event is recognised as a no-op.
    pub codex_watcher_handle: Arc<RwLock<Option<WatcherHandle>>>,
    /// Liveness view of the root supervisor's tasks, read by `/livez`.
    ///
    /// Non-empty means a task declared [`Criticality::Fatal`] or
    /// [`Criticality::Liveness`] terminated before shutdown; see
    /// [`SandboxState::is_live`].
    ///
    /// [`Criticality::Fatal`]: crate::supervisor::Criticality::Fatal
    /// [`Criticality::Liveness`]: crate::supervisor::Criticality::Liveness
    pub supervisor_health: Arc<SupervisorHealth>,
    /// Interactive PTY sessions exposed through the T3 compatibility surface.
    pub terminals: TerminalManager,
    /// Fingerprint of the last workspace successfully uploaded to S3, shared by
    /// every upload path (turn boundary, the backend's pre-stop `SyncWorkspace`,
    /// the shutdown flush, the periodic loop) so none of them re-ships a tar
    /// another already shipped.
    pub workspace_sync: WorkspaceSyncTracker,
}

impl SandboxState {
    /// Resolve a task context from an optional task id.
    ///
    /// Omitted task ids mean "the pod's only member", preserving the historical
    /// single-task gRPC contract.
    pub async fn task_context(
        &self,
        task_id: Option<&str>,
    ) -> Result<Arc<TaskContext>, TaskContextLookupError> {
        let contexts = self.task_contexts.read().await;
        match task_id {
            Some(task_id) => contexts
                .get(task_id)
                .cloned()
                .ok_or_else(|| TaskContextLookupError::NotFound(task_id.to_string())),
            None => match contexts.len() {
                0 => Err(TaskContextLookupError::Empty),
                1 => contexts
                    .values()
                    .next()
                    .cloned()
                    .ok_or(TaskContextLookupError::Empty),
                _ => Err(TaskContextLookupError::Ambiguous),
            },
        }
    }

    /// Snapshot registered contexts without holding the map lock across awaits.
    pub async fn task_contexts_snapshot(&self) -> Vec<Arc<TaskContext>> {
        self.task_contexts.read().await.values().cloned().collect()
    }

    /// Register a task as a member of this sandbox process.
    ///
    /// Idempotent per task: replaying `JoinTask` after a mid-join failure
    /// returns the existing context and does not spawn another agent loop.
    pub async fn register_task_context(
        &self,
        task_id: String,
        checkout_mode: Option<&str>,
    ) -> Result<Arc<TaskContext>, RegisterTaskContextError> {
        if task_id.is_empty() {
            return Err(RegisterTaskContextError::EmptyTaskId);
        }

        {
            let contexts = self.task_contexts.read().await;
            if let Some(context) = contexts.get(&task_id) {
                return Ok(Arc::clone(context));
            }
        }

        let checkout_mode = CheckoutMode::parse(checkout_mode)?;
        let _registration = self.task_context_registration.lock().await;
        {
            let contexts = self.task_contexts.read().await;
            if let Some(context) = contexts.get(&task_id) {
                return Ok(Arc::clone(context));
            }
        }

        let (workspace_dir, checkout_dir) = self
            .materialize_checkout(&task_id, checkout_mode)
            .await
            .map_err(RegisterTaskContextError::CheckoutMaterialization)?;
        let mut task_config = (*self.config).clone();
        task_config.task_id = task_id.clone();
        task_config.workspace_dir = workspace_dir;
        task_config.repo_dir = checkout_dir.clone();
        let task_config = Arc::new(task_config);

        let agent_state = Arc::new(AgentState::new());
        let late_injector = Arc::new(LateInjector::new());
        let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::clone(&late_injector) as _));
        let commands = Arc::new(
            CommandRegistry::new(
                Duration::from_secs(task_config.command_default_timeout_secs),
                crate::commands::default_log_dir(&task_id),
            )
            .with_journal(crate::commands::default_journal_path(
                &task_config.workspace_dir,
            ))
            .with_liveness(self.events.clone(), task_id.clone()),
        );
        commands.report_initial_liveness().await;
        let credential_failures = CredentialFailureDetector::default();
        let system_prompt_cache: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));

        let agent_handle = spawn_agent(
            Arc::clone(&task_config),
            Arc::clone(&agent_state),
            self.events.clone(),
            Arc::clone(&self.credentials),
            Arc::clone(&self.backend_client),
            Arc::clone(&scheduled_jobs),
            Arc::clone(&commands),
            credential_failures.clone(),
            Arc::clone(&self.credential_dispatcher),
            Arc::clone(&system_prompt_cache),
            self.heartbeat.clone(),
            self.workspace_sync.clone(),
        );
        late_injector.set(Box::new(agent_handle.clone()));

        let context = Arc::new(TaskContext::new(
            task_id.clone(),
            task_config.workspace_dir.clone(),
            checkout_dir,
            checkout_mode,
            agent_state,
            agent_handle,
            scheduled_jobs,
            commands,
            credential_failures,
            system_prompt_cache,
        ));
        let mut contexts = self.task_contexts.write().await;
        contexts.insert(task_id, Arc::clone(&context));
        Ok(context)
    }

    async fn materialize_checkout(
        &self,
        task_id: &str,
        checkout_mode: CheckoutMode,
    ) -> Result<(String, String), String> {
        match checkout_mode {
            CheckoutMode::Shared => Ok((
                self.config.workspace_dir.clone(),
                self.config.repo_dir.clone(),
            )),
            CheckoutMode::Isolated => self.materialize_isolated_checkout(task_id).await,
        }
    }

    async fn materialize_isolated_checkout(
        &self,
        task_id: &str,
    ) -> Result<(String, String), String> {
        let workspace_dir = isolated_workspace_dir(&self.config.workspace_dir, task_id)?;
        tokio::fs::create_dir_all(&workspace_dir)
            .await
            .map_err(|e| format!("create isolated workspace {workspace_dir}: {e}"))?;

        let mut task_config = (*self.config).clone();
        task_config.task_id = task_id.to_string();
        task_config.workspace_dir = workspace_dir.clone();
        task_config.repo_dir = primary_repo_dir_for_workspace(&task_config);

        if !task_config.repos.is_empty() {
            if let Err(e) = crate::init::git::clone_repositories(&task_config).await {
                if let Err(cleanup) = tokio::fs::remove_dir_all(&workspace_dir).await {
                    warn!(
                        task_id = %task_id,
                        dir = %workspace_dir,
                        error = %cleanup,
                        "Failed to clean isolated checkout after clone failure"
                    );
                }
                return Err(e.to_string());
            }
        }

        Ok((workspace_dir, task_config.repo_dir))
    }

    /// The `/livez` decision: is this pod still able to make progress?
    ///
    /// Two independent ways to lose that, both of which want the same remedy —
    /// replace the pod:
    ///
    /// 1. A supervised task the pod needs terminated early. The supervisor
    ///    records it by name; nothing here is restartable in place.
    /// 2. The agent lifecycle task wedged or exited — see
    ///    [`crate::agent::heartbeat::actor_is_alive`].
    ///
    /// Deliberately still independent of backend reachability, credentials, and
    /// disk: a supervised task's termination is *this process* being broken, not
    /// a dependency being briefly unavailable, so a transient outage still
    /// cannot trigger a restart that kills an in-flight turn.
    pub async fn is_live(&self) -> bool {
        use crate::agent::heartbeat::{HEARTBEAT_STALE_THRESHOLD_MS, actor_is_alive, now_millis};
        use crate::agent::types::AgentStatus;

        let failed = self.supervisor_health.failed_tasks();
        if !failed.is_empty() {
            warn!(?failed, "Liveness: supervised tasks terminated early");
            return false;
        }

        let contexts = self.task_contexts_snapshot().await;
        if contexts.is_empty() {
            warn!("Liveness: no task contexts registered");
            return false;
        }

        for context in contexts {
            let turn_running = matches!(context.agent.status().await, AgentStatus::Running);
            if !actor_is_alive(
                now_millis(),
                self.heartbeat.last_beat_ms(),
                turn_running,
                !context.agent_handle.is_closed(),
                HEARTBEAT_STALE_THRESHOLD_MS,
            ) {
                warn!(task_id = %context.task_id, "Liveness: agent lifecycle unhealthy");
                return false;
            }
        }

        true
    }

    /// Whether core init has finished (Session state restored, workspace cloned)
    /// and the pod has reached [`InitPhase::Ready`].
    ///
    /// The message dispatcher gates on this so a backend notification that races
    /// init (the gRPC server accepts calls before `run_init` completes) can never
    /// drive the harness before restoration — `run_init` performs the
    /// authoritative pending dispatch itself once Ready.
    pub(crate) async fn is_ready(&self) -> bool {
        matches!(*self.init_phase.read().await, InitPhase::Ready)
    }
}

fn isolated_workspace_dir(root_workspace_dir: &str, task_id: &str) -> Result<String, String> {
    let segment = sanitize_mount_name(task_id)
        .ok_or_else(|| format!("task_id cannot form a safe checkout directory: {task_id}"))?;
    Ok(Path::new(root_workspace_dir)
        .join(".moatless")
        .join("members")
        .join(segment)
        .to_string_lossy()
        .into_owned())
}

fn primary_repo_dir_for_workspace(config: &Config) -> String {
    config
        .repos
        .iter()
        .find(|repo| repo.is_primary)
        .or_else(|| config.repos.first())
        .map_or_else(
            || config.workspace_dir.clone(),
            |repo| repo.dir(&config.workspace_dir),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    use sandbox_proto::{
        DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery,
        PublishEventRequest,
    };

    use crate::agent::handle::AgentHandle;
    use crate::config::test_support::minimal_config;
    use crate::outbox::EventOutbox;

    struct NoopBackend;

    #[async_trait::async_trait]
    impl BackendClient for NoopBackend {
        async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn get_pending_messages(
            &self,
            _task_id: &str,
        ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
            Ok(vec![])
        }

        async fn acknowledge_messages(
            &self,
            _task_id: &str,
            _message_uuids: Vec<String>,
            _session_id: &str,
            _run_id: &str,
            _answered_prompt: Option<&str>,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn get_credentials(
            &self,
            _task_id: &str,
        ) -> Result<GetCredentialsResponse, tonic::Status> {
            Ok(GetCredentialsResponse::default())
        }

        async fn update_credential(
            &self,
            _task_id: &str,
            _key: &str,
            _value: &str,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
            Ok(String::new())
        }

        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<DownloadUploadResponse, tonic::Status> {
            Err(tonic::Status::not_found("no uploads in this fixture"))
        }

        async fn recover_interrupted_run(
            &self,
            _task_id: &str,
        ) -> Result<Option<InterruptedTurn>, tonic::Status> {
            Ok(None)
        }

        async fn upload_transcript(
            &self,
            _task_id: &str,
            _tar_data: Vec<u8>,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, tonic::Status> {
            Ok(Vec::new())
        }
    }

    fn test_state() -> SandboxState {
        let mut config = minimal_config();
        config.task_id = "task-1".to_string();
        config.sandbox_id = "sandbox-1".to_string();
        config.repo_dir = "/tmp/repo".to_string();
        config.agent_type = Some("claude-code".to_string());
        config.claude_cli_path = "/missing/claude".to_string();
        config.codex_cli_path = "/missing/codex".to_string();
        let config = Arc::new(config);
        let backend_client: Arc<dyn BackendClient> = Arc::new(NoopBackend);
        let lifecycle_cancel = CancellationToken::new();
        let (outbox, _drainer) = EventOutbox::spawn(
            Arc::clone(&backend_client),
            config.task_id.clone(),
            lifecycle_cancel.clone(),
        );
        let events = GrpcEventPublisher::new(outbox);
        let (agent_handle, _rx) = AgentHandle::test_channel();
        let initial_context = Arc::new(TaskContext::new(
            config.task_id.clone(),
            config.workspace_dir.clone(),
            config.repo_dir.clone(),
            CheckoutMode::Shared,
            Arc::new(AgentState::new()),
            agent_handle,
            Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
            Arc::new(CommandRegistry::for_test()),
            CredentialFailureDetector::default(),
            Arc::new(RwLock::new(None)),
        ));

        SandboxState {
            config,
            events,
            task_contexts: Arc::new(RwLock::new(HashMap::from([(
                initial_context.task_id.clone(),
                initial_context,
            )]))),
            task_context_registration: Arc::new(Mutex::new(())),
            heartbeat: Heartbeat::new(),
            backend_client,
            init_phase: Arc::new(RwLock::new(InitPhase::Ready)),
            credentials: Arc::new(RwLock::new(HashMap::new())),
            lifecycle_cancel,
            credential_dispatcher: Arc::new(Dispatcher::new()),
            codex_watcher_handle: Arc::new(RwLock::new(None)),
            supervisor_health: Default::default(),
            terminals: TerminalManager::new("/tmp/workspace".to_string()),
            workspace_sync: Default::default(),
        }
    }

    #[tokio::test]
    async fn register_task_context_adds_a_joined_member() {
        let state = test_state();

        let joined = state
            .register_task_context("task-2".to_string(), Some("shared"))
            .await
            .expect("join registers");

        assert_eq!(joined.task_id, "task-2");
        assert_eq!(joined.checkout_mode, CheckoutMode::Shared);
        assert!(state.task_context(Some("task-2")).await.is_ok());
        assert_eq!(state.task_contexts_snapshot().await.len(), 2);
    }

    #[tokio::test]
    async fn register_task_context_is_idempotent_per_task() {
        let state = test_state();

        let first = state
            .register_task_context("task-2".to_string(), Some("shared"))
            .await
            .expect("first join registers");
        let second = state
            .register_task_context("task-2".to_string(), Some("shared"))
            .await
            .expect("second join reuses");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(state.task_contexts_snapshot().await.len(), 2);
    }

    #[tokio::test]
    async fn register_task_context_rejects_unknown_checkout_mode() {
        let state = test_state();

        let Err(err) = state
            .register_task_context("task-2".to_string(), Some("exclusive"))
            .await
        else {
            panic!("unknown checkout mode is rejected");
        };

        assert!(matches!(
            err,
            RegisterTaskContextError::InvalidCheckoutMode(mode) if mode == "exclusive"
        ));
    }

    #[tokio::test]
    async fn register_task_context_materializes_isolated_checkout() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state = test_state();
        let mut config = (*state.config).clone();
        config.workspace_dir = tmp.path().join("workspace").display().to_string();
        config.repo_dir = tmp.path().join("workspace/shared").display().to_string();
        config.repos = Vec::new();
        let state = SandboxState {
            config: Arc::new(config),
            ..state
        };

        let joined = state
            .register_task_context("task-2".to_string(), Some("isolated"))
            .await
            .expect("isolated join registers");

        assert_eq!(joined.checkout_mode, CheckoutMode::Isolated);
        assert_ne!(joined.workspace_dir, state.config.workspace_dir);
        assert_ne!(joined.checkout_dir, state.config.repo_dir);
        assert_eq!(joined.checkout_dir, joined.workspace_dir);
        assert!(joined.checkout_dir.ends_with(".moatless/members/task-2"));
        assert!(std::path::Path::new(&joined.workspace_dir).is_dir());
        assert!(std::path::Path::new(&joined.checkout_dir).is_dir());
    }

    #[tokio::test]
    async fn omitted_task_id_is_ambiguous_after_join() {
        let state = test_state();
        state
            .register_task_context("task-2".to_string(), Some("shared"))
            .await
            .expect("join registers");

        let Err(err) = state.task_context(None).await else {
            panic!("multiple contexts require task_id");
        };

        assert!(matches!(err, TaskContextLookupError::Ambiguous));
    }
}
