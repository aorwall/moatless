//! Shared application state for the sandbox service.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::heartbeat::Heartbeat;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::Dispatcher;
use crate::credentials::codex_watcher::WatcherHandle;
use crate::events::GrpcEventPublisher;
use crate::init::{InitPhase, WorkspaceOrigin};
use crate::supervisor::SupervisorHealth;
use crate::terminal::TerminalManager;
use crate::workspace_sync::WorkspaceSyncTracker;
use crate::writer_admission::WriterAdmission;

pub use crate::task_context::{
    CheckoutMode, RegisterTaskContextError, TaskContext, TaskContextLookupError,
};

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
    /// `/livez` probe to detect a wedged service (see
    /// [`crate::agent::heartbeat`]).
    pub heartbeat: Heartbeat,
    /// Backend client for gRPC communication (message polling, acknowledgment).
    pub backend_client: Arc<dyn BackendClient>,
    /// Current initialization phase — tracked for health probes and consumer
    /// gating.
    pub init_phase: Arc<RwLock<InitPhase>>,
    /// Holds agent turns until workspace setup has run.
    pub setup_gate: crate::init::SetupGate,
    /// Written once by `agent::restart::open_for_work` under
    /// `task_context_registration`.
    ///
    /// A joining Member prepends the same `<sandbox-restart-note>` as the boot
    /// Member — every checkout shares the pod's one volume.
    pub workspace_origin: Arc<RwLock<WorkspaceOrigin>>,
    /// Credentials fetched during init, passed to the agent subprocess via
    /// `ClaudeOptions.env`, except keys in
    /// [`crate::credentials::fetch::AGENT_EXCLUDED_KEYS`].
    pub credentials: Arc<RwLock<HashMap<String, String>>>,
    /// The git credentials from the latest fetch, handed to git setup and
    /// clones; never exported into the process env.
    pub git_credentials: Arc<RwLock<crate::init::git::GitCredentials>>,
    /// Cancellation token tied to the sandbox lifecycle.
    ///
    /// Used by background tasks (e.g. the credential refresher) to shut down
    /// cleanly when the pod is stopping.
    pub lifecycle_cancel: CancellationToken,
    /// Generic dispatcher for backend-pushed credential refreshes.
    ///
    /// The gRPC server hands the rotated value to this dispatcher, which routes
    /// to the appropriate sink (file write, env var update, etc.).
    pub credential_dispatcher: Arc<Dispatcher>,
    /// Handle to the Codex auth watcher, used by [`crate::credentials::sinks`]
    /// to seed the watcher cache after a backend-driven write so the resulting
    /// filesystem event is recognised as a no-op.
    pub codex_watcher_handle: Arc<RwLock<Option<WatcherHandle>>>,
    /// Liveness view of the root supervisor's tasks, read by `/livez`.
    ///
    /// Non-empty means a task declared `Criticality::Fatal` or
    /// `Criticality::Liveness` terminated before shutdown; see
    /// [`SandboxState::is_live`].
    pub supervisor_health: Arc<SupervisorHealth>,
    /// Interactive PTY sessions exposed through the T3 compatibility surface.
    pub terminals: TerminalManager,
    /// Fingerprint of the last workspace successfully uploaded to S3.
    ///
    /// Shared by every upload path (turn boundary, the backend's pre-stop
    /// `SyncWorkspace`, the shutdown flush, the periodic loop) so none re-ships
    /// a tar another already shipped.
    pub workspace_sync: WorkspaceSyncTracker,
    /// Admission for every workspace writer this pod can start, closed once by
    /// [`crate::shutdown::perform_shutdown_for_state`].
    pub writer_admission: Arc<WriterAdmission>,
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

    /// Register a task once, materializing its checkout before publishing
    /// membership.
    pub async fn register_task_context(
        &self,
        task_id: String,
        checkout_mode: Option<&str>,
        agent_profile: Option<&str>,
    ) -> Result<Arc<TaskContext>, RegisterTaskContextError> {
        crate::task_context::register(self, task_id, checkout_mode, agent_profile).await
    }

    /// False when [`SandboxState::supervisor_health`] records a failure or
    /// [`crate::agent::heartbeat::actor_is_alive`] fails.
    ///
    /// Deliberately independent of backend reachability, credentials, and disk
    /// — a transient outage there must not kill a turn in flight.
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

    /// True once session state is restored and the workspace is cloned.
    ///
    /// The message dispatcher gates on this so a backend notification racing
    /// init can never drive the harness before restoration; `run_init` performs
    /// the authoritative pending dispatch itself once Ready.
    pub(crate) async fn is_ready(&self) -> bool {
        matches!(*self.init_phase.read().await, InitPhase::Ready)
    }
}

/// Builds a [`SandboxState`] over fakes, for tests of flows that take the whole
/// state rather than a config. The caller owns `config` and `backend_client`;
/// everything else is a default or a test channel.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::agent::agent_state::AgentState;
    use crate::agent::handle::AgentHandle;
    use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
    use crate::commands::CommandRegistry;
    use crate::config::test_support::minimal_config;
    use crate::credentials::recovery::CredentialFailureDetector;
    use crate::outbox::EventOutbox;
    use sandbox_proto::{DownloadUploadResponse, PendingAgentDelivery};

    /// Stores nothing. Its first `failing_fetches` reads of pending messages
    /// fail as unavailable; `fetches` counts every read.
    #[derive(Default)]
    pub(crate) struct NoopBackend {
        pub(crate) fetches: std::sync::atomic::AtomicUsize,
        pub(crate) failing_fetches: usize,
    }

    #[async_trait::async_trait]
    impl BackendClient for NoopBackend {
        async fn get_pending_messages(
            &self,
            _task_id: &str,
        ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
            let fetch = self
                .fetches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fetch < self.failing_fetches {
                return Err(tonic::Status::unavailable("the backend is restarting"));
            }
            Ok(vec![])
        }

        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<DownloadUploadResponse, tonic::Status> {
            Err(tonic::Status::not_found("no uploads in this fixture"))
        }
    }

    /// [`state_with`] over a backend that stores nothing and answers every call
    /// successfully.
    pub(crate) fn state_for_test() -> SandboxState {
        let mut config = minimal_config();
        config.task_id = "task-1".to_string();
        config.sandbox_id = "sandbox-1".to_string();
        config.repo_dir = "/tmp/repo".to_string();
        config.agent_profile = Some("claude-code".to_string());
        config.claude_cli_path = "/missing/claude".to_string();
        config.codex_cli_path = "/missing/codex".to_string();
        state_with(config, Arc::new(NoopBackend::default()))
    }

    pub(crate) fn state_with(
        config: Config,
        backend_client: Arc<dyn BackendClient>,
    ) -> SandboxState {
        let config = Arc::new(config);
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
            config.agent_profile.clone(),
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
            setup_gate: crate::init::SetupGate::opened(),
            workspace_origin: Default::default(),
            git_credentials: Default::default(),
            credentials: Arc::new(RwLock::new(HashMap::new())),
            lifecycle_cancel,
            credential_dispatcher: Arc::new(Dispatcher::new()),
            codex_watcher_handle: Arc::new(RwLock::new(None)),
            supervisor_health: Default::default(),
            terminals: TerminalManager::new("/tmp/workspace".to_string()),
            workspace_sync: Default::default(),
            writer_admission: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::state_for_test as test_state;
    use super::*;

    #[tokio::test]
    async fn register_task_context_adds_a_joined_member() {
        let state = test_state();

        let joined = state
            .register_task_context("task-2".to_string(), Some("shared"), None)
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
            .register_task_context("task-2".to_string(), Some("shared"), None)
            .await
            .expect("first join registers");
        let second = state
            .register_task_context("task-2".to_string(), Some("shared"), None)
            .await
            .expect("second join reuses");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(state.task_contexts_snapshot().await.len(), 2);
    }

    /// The regression this guards: the pod boots `claude-code`, a Codex Task
    /// joins it, and the joined Member's harness must be Codex.
    ///
    /// Building it from `AGENT_PROFILE` instead sends those Turns to Claude CLI,
    /// which kills the turn in half a second, surfacing nothing to the sender.
    #[tokio::test]
    async fn joined_member_harness_comes_from_the_join_not_the_pod() {
        let state = test_state();
        assert_eq!(state.config.agent_profile.as_deref(), Some("claude-code"));

        let joined = state
            .register_task_context("task-2".to_string(), Some("shared"), Some("codex"))
            .await
            .expect("join registers");

        assert_eq!(joined.agent_profile.as_deref(), Some("codex"));
        // The boot Member keeps its own.
        let boot = state
            .task_context(Some("task-1"))
            .await
            .expect("boot member");
        assert_eq!(boot.agent_profile.as_deref(), Some("claude-code"));
    }

    /// A caller that predates `JoinTaskRequest.agent_profile` leaves the Member on
    /// the pod's own selection, which is what it had before the field existed.
    #[tokio::test]
    async fn joined_member_without_an_agent_profile_inherits_the_pod() {
        let state = test_state();

        let joined = state
            .register_task_context("task-2".to_string(), Some("shared"), None)
            .await
            .expect("join registers");

        assert_eq!(joined.agent_profile.as_deref(), Some("claude-code"));
    }

    /// Refused at the join rather than at `spawn_agent`, which answers an
    /// unsupported harness by closing the Member's command channel — and
    /// `is_live` reads every Member's, so one bad join would replace the pod.
    #[tokio::test]
    async fn register_task_context_rejects_unsupported_agent_profile() {
        let state = test_state();

        let Err(err) = state
            .register_task_context("task-2".to_string(), Some("shared"), Some("gpt-9000"))
            .await
        else {
            panic!("unsupported agent profile is rejected");
        };

        let RegisterTaskContextError::UnsupportedAgentProfile(diagnostic) = &err else {
            panic!("unsupported agent profile is rejected as such, got {err}");
        };
        assert!(diagnostic.contains("gpt-9000"), "{diagnostic}");
        // The accepted set travels with the rejection: the reconciler retries
        // the join every 30s, and its log line is where this is read.
        assert!(diagnostic.contains("codex"), "{diagnostic}");
        // No context was composed, so no closed command channel is left behind
        // for `is_live` to read.
        assert!(state.task_context(Some("task-2")).await.is_err());
        assert_eq!(state.task_contexts_snapshot().await.len(), 1);
    }

    #[tokio::test]
    async fn register_task_context_rejects_unknown_checkout_mode() {
        let state = test_state();

        let Err(err) = state
            .register_task_context("task-2".to_string(), Some("exclusive"), None)
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
            .register_task_context("task-2".to_string(), Some("isolated"), None)
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
            .register_task_context("task-2".to_string(), Some("shared"), None)
            .await
            .expect("join registers");

        let Err(err) = state.task_context(None).await else {
            panic!("multiple contexts require task_id");
        };

        assert!(matches!(err, TaskContextLookupError::Ambiguous));
    }
}
