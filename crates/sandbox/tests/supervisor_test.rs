//! Root-supervision behaviour for the sandbox runtime.
//!
//! Two groups:
//!
//! 1. [`Supervisor`] policy — what a task's declared [`Criticality`] does when
//!    it terminates, and that the shutdown join is bounded.
//! 2. The shutdown *ordering* the supervisor must not disturb: the event outbox
//!    is flushed while its drainer is still alive, and only then is the
//!    lifecycle token cancelled.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery,
    PublishEventRequest,
};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use moatless_sandbox::agent::agent_state::AgentState;
use moatless_sandbox::agent::handle::spawn_agent;
use moatless_sandbox::agent::heartbeat::Heartbeat;
use moatless_sandbox::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
use moatless_sandbox::backend_client::BackendClient;
use moatless_sandbox::config::{Config, WorkspaceSnapshotFormat};
use moatless_sandbox::credentials::Dispatcher;
use moatless_sandbox::credentials::recovery::CredentialFailureDetector;
use moatless_sandbox::events::GrpcEventPublisher;
use moatless_sandbox::init::InitPhase;
use moatless_sandbox::outbox::EventOutbox;
use moatless_sandbox::state::{SandboxState, TaskContext};
use moatless_sandbox::supervisor::{Criticality, Supervisor};

/// Long enough that a test never trips it by accident, short enough that the
/// straggler case below still finishes quickly.
const JOIN_DEADLINE: Duration = Duration::from_millis(500);

/// No graceful teardown. Used by the policy tests, which assert what a
/// `Criticality` does rather than the shutdown ordering — that invariant is
/// owned by the two `*_before_cancelling` tests at the bottom of this file.
fn no_teardown() -> std::future::Ready<()> {
    std::future::ready(())
}

// =============================================================================
// Supervisor policy
// =============================================================================

#[tokio::test]
async fn fatal_early_return_fails_the_process_and_affects_liveness() {
    // Shaped like the gRPC server: `serve` returns on its own before any
    // shutdown was requested. Nothing else notices, which is the bug.
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel.clone());
    let health = supervisor.health();
    supervisor
        .registrar()
        .supervise("grpc-server", Criticality::Fatal, tokio::spawn(async {}));

    let exit = supervisor
        .run(JOIN_DEADLINE, no_teardown())
        .await
        .expect_err("a Fatal task returning early must fail `main`");

    assert_eq!(exit.name, "grpc-server");
    // ...and the failure is visible on `/livez` while the process winds down,
    // rather than the probe claiming a liveness the pod no longer has.
    assert_eq!(health.failed_tasks(), vec!["grpc-server"]);
    // A fatal exit cancels the root token, so every other task winds down too.
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn fatal_panic_carries_the_panic_message() {
    let supervisor = Supervisor::new(CancellationToken::new());
    supervisor.registrar().supervise(
        "grpc-server",
        Criticality::Fatal,
        tokio::spawn(async { panic!("bind failed") }),
    );

    let exit = supervisor
        .run(JOIN_DEADLINE, no_teardown())
        .await
        .unwrap_err();
    assert!(
        exit.to_string().contains("bind failed"),
        "panic message must survive into the process's exit error: {exit}"
    );
}

#[tokio::test]
async fn liveness_failure_keeps_serving_but_flips_livez() {
    // Shaped like a durability loop: it dies, the pod keeps running the turn it
    // has, and `/livez` reports the pod as replaceable.
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel.clone());
    let health = supervisor.health();
    let registrar = supervisor.registrar();
    registrar.supervise(
        "transcript-sync",
        Criticality::Liveness,
        tokio::spawn(async { panic!("sync loop blew up") }),
    );
    // A second task that only stops on cancellation, so `run` cannot return
    // merely because it ran out of work.
    let live_cancel = cancel.clone();
    registrar.supervise(
        "grpc-server",
        Criticality::Fatal,
        tokio::spawn(async move { live_cancel.cancelled().await }),
    );

    // The Liveness failure must NOT end the supervisor: cancel externally.
    let cancel_later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_later.cancel();
    });

    supervisor
        .run(JOIN_DEADLINE, no_teardown())
        .await
        .expect("a Liveness failure must not fail the process");
    assert_eq!(health.failed_tasks(), vec!["transcript-sync"]);
}

#[tokio::test]
async fn optional_failure_is_invisible_to_liveness() {
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel.clone());
    let health = supervisor.health();
    supervisor.registrar().supervise(
        "codex-auth-watcher",
        Criticality::Optional,
        tokio::spawn(async { panic!("watcher blew up") }),
    );

    let cancel_later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_later.cancel();
    });

    supervisor.run(JOIN_DEADLINE, no_teardown()).await.unwrap();
    assert!(
        health.failed_tasks().is_empty(),
        "losing an Optional task must not make the pod restart-worthy"
    );
}

#[tokio::test]
async fn initialization_join_failure_fails_startup() {
    // The criterion: a panic inside `run_init` must fail the process instead of
    // disappearing with a discarded `JoinHandle`.
    let supervisor = Supervisor::new(CancellationToken::new());
    supervisor.registrar().supervise_finite(
        "init",
        Criticality::Fatal,
        tokio::spawn(async { panic!("core init exploded") }),
    );

    let exit = supervisor
        .run(JOIN_DEADLINE, no_teardown())
        .await
        .unwrap_err();
    assert_eq!(exit.name, "init");
    assert!(exit.to_string().contains("core init exploded"));
}

#[tokio::test]
async fn initialization_completing_is_not_a_failure() {
    // The other half: `run_init` returning is the normal end of startup. A
    // supervisor that treated every early return as a crash would kill every
    // pod the moment it finished initializing.
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel.clone());
    let health = supervisor.health();
    supervisor
        .registrar()
        .supervise_finite("init", Criticality::Fatal, tokio::spawn(async {}));

    let cancel_later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_later.cancel();
    });

    supervisor
        .run(JOIN_DEADLINE, no_teardown())
        .await
        .expect("a finite task completing cleanly is success");
    assert!(health.failed_tasks().is_empty());
}

#[tokio::test]
async fn periodic_task_stopping_on_cancellation_is_not_a_failure() {
    // Every durability loop exits by observing the lifecycle token. That is the
    // normal shutdown path and must never be reported as an early termination.
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel.clone());
    let health = supervisor.health();
    let registrar = supervisor.registrar();

    for name in ["credential-refresher", "transcript-sync", "workspace-sync"] {
        let task_cancel = cancel.clone();
        registrar.supervise(
            name,
            Criticality::Liveness,
            tokio::spawn(async move { task_cancel.cancelled().await }),
        );
    }

    let cancel_later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_later.cancel();
    });

    supervisor.run(JOIN_DEADLINE, no_teardown()).await.unwrap();
    assert!(
        health.failed_tasks().is_empty(),
        "cancellation is shutdown, not failure: {:?}",
        health.failed_tasks()
    );
}

#[tokio::test]
async fn shutdown_join_is_bounded_by_the_deadline() {
    // A task that ignores cancellation must not hold the process past the
    // deadline — that is the exit-137 kill the shutdown budget exists to avoid.
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel.clone());
    supervisor.registrar().supervise(
        "wedged-task",
        Criticality::Liveness,
        tokio::spawn(async { std::future::pending::<()>().await }),
    );

    cancel.cancel();
    let started = std::time::Instant::now();
    supervisor.run(JOIN_DEADLINE, no_teardown()).await.unwrap();
    let elapsed = started.elapsed();

    assert!(
        elapsed < JOIN_DEADLINE * 4,
        "join must be bounded, took {elapsed:?}"
    );
}

#[tokio::test]
async fn late_registration_is_supervised_too() {
    // The sandbox's durability loops are spawned from inside `run_init`, which
    // is itself supervised — so registration has to work after `run` starts.
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel.clone());
    let health = supervisor.health();
    let registrar = supervisor.registrar();

    let late_cancel = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        registrar.supervise(
            "transcript-sync",
            Criticality::Liveness,
            tokio::spawn(async { panic!("late loop blew up") }),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        late_cancel.cancel();
    });

    supervisor.run(JOIN_DEADLINE, no_teardown()).await.unwrap();
    assert_eq!(health.failed_tasks(), vec!["transcript-sync"]);
}

// =============================================================================
// `/livez` composition
// =============================================================================

#[tokio::test]
async fn livez_reports_not_live_after_a_supervised_task_dies() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = Arc::new(RecordingBackend::default());
    let state = test_state(Arc::clone(&backend) as _, &dir);
    assert!(state.is_live().await, "a fresh pod is live");

    // Exactly what the supervisor does when the gRPC server terminates early.
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel.clone());
    supervisor
        .registrar()
        .supervise("grpc-server", Criticality::Fatal, tokio::spawn(async {}));
    let health = supervisor.health();
    supervisor
        .run(JOIN_DEADLINE, no_teardown())
        .await
        .unwrap_err();

    let state = SandboxState {
        supervisor_health: health,
        ..state
    };
    assert!(
        !state.is_live().await,
        "early gRPC termination must be visible on /livez"
    );
}

// =============================================================================
// Shutdown ordering: flush while the drainer is alive, cancel afterwards
// =============================================================================

/// The whole point of the outbox flush on shutdown: an event still owed to the
/// backend when SIGTERM lands is delivered, and it is delivered *before* the
/// lifecycle token is cancelled.
///
/// This pins the position of `shutdown::perform_shutdown`'s flush, not its
/// duration. `EventOutbox::flush` early-returns as soon as the drainer has
/// stopped, so any change that cancels the lifecycle token before or alongside
/// the flush makes the flush an instant no-op that reports success while
/// dropping the pod's terminal lifecycle events. Without this test that
/// regression is invisible: nothing errors and nothing else goes red.
#[tokio::test]
async fn shutdown_flushes_the_outbox_before_cancelling() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = Arc::new(RecordingBackend::default());
    // Fail the first publish so the event is still owed when shutdown starts.
    backend.fail_first_publish();

    let state = test_state(Arc::clone(&backend) as _, &dir);
    let lifecycle_cancel = state.lifecycle_cancel.clone();
    // From here on the backend records, per publish, whether this token was
    // already cancelled — the signal that distinguishes a real flush from one
    // that early-returned on a dead drainer.
    backend.watch_lifecycle(lifecycle_cancel.clone());

    state.events.outbox().enqueue(event("agent.stopped"));

    // Wait for the failing attempt: the drainer is now parked in its retry
    // backoff holding one undrained event — precisely the state the shutdown
    // flush exists to resolve, and the state in which a prematurely-cancelled
    // drainer loses the event.
    wait_until(|| backend.publish_attempts() >= 1).await;
    assert_eq!(backend.delivered().len(), 0);

    let context = state
        .task_context(None)
        .await
        .expect("test state has one task context");
    moatless_sandbox::shutdown::perform_shutdown(&context.agent, &lifecycle_cancel, Some(&state))
        .await;

    assert_eq!(
        backend.delivered(),
        vec!["agent.stopped".to_string()],
        "the shutdown flush must drain the event the drainer still owed"
    );
    assert!(
        backend.all_publishes_saw_a_live_lifecycle(),
        "the flush must run while the drainer is alive — the lifecycle token \
         was already cancelled when the event was published, which means the \
         flush was a no-op and only luck delivered it"
    );
    assert!(
        lifecycle_cancel.is_cancelled(),
        "shutdown still cancels the lifecycle token, just last"
    );
}

/// The same invariant on the *fatal* path, where no SIGTERM arrives and the
/// supervisor itself initiates shutdown. See [`Supervisor::run`] for the
/// teardown-before-cancel ordering this guards. Same fixture as above, one
/// different trigger.
#[tokio::test]
async fn fatal_exit_flushes_the_outbox_before_cancelling() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = Arc::new(RecordingBackend::default());
    backend.fail_first_publish();

    let state = test_state(Arc::clone(&backend) as _, &dir);
    let lifecycle_cancel = state.lifecycle_cancel.clone();
    backend.watch_lifecycle(lifecycle_cancel.clone());

    state.events.outbox().enqueue(event("agent.stopped"));
    wait_until(|| backend.publish_attempts() >= 1).await;
    assert_eq!(backend.delivered().len(), 0);

    // A Fatal task that is NOT the drainer: the backend connection is fine, so
    // the terminal event both can and must still be delivered.
    let supervisor = Supervisor::new(lifecycle_cancel.clone());
    supervisor
        .registrar()
        .supervise("grpc-server", Criticality::Fatal, tokio::spawn(async {}));

    let context = state
        .task_context(None)
        .await
        .expect("test state has one task context");
    let teardown = moatless_sandbox::shutdown::perform_shutdown(
        &context.agent,
        &lifecycle_cancel,
        Some(&state),
    );
    let exit = supervisor
        .run(JOIN_DEADLINE, teardown)
        .await
        .expect_err("the Fatal task must still fail the process");
    assert_eq!(exit.name, "grpc-server");

    assert_eq!(
        backend.delivered(),
        vec!["agent.stopped".to_string()],
        "a fatal exit must still flush the events the drainer owed — that is \
         how the backend learns this sandbox died rather than inferring it"
    );
    assert!(
        backend.all_publishes_saw_a_live_lifecycle(),
        "the supervisor cancelled before running the teardown, so the flush \
         early-returned on a stopped drainer and delivered nothing"
    );
    assert!(lifecycle_cancel.is_cancelled());
}

/// Poll a condition to a bounded limit. Cheaper and more deterministic than a
/// fixed sleep: the assertions above fail with a real message if it never holds.
async fn wait_until(mut condition: impl FnMut() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition never held within 2s");
}

fn event(sub_type: &str) -> PublishEventRequest {
    PublishEventRequest {
        task_id: "test-task".to_string(),
        event_type: sandbox_proto::EventType::Agent as i32,
        sub_type: sub_type.to_string(),
        payload_json: "{}".to_string(),
        event_id: String::new(),
        sequence: 0,
        run_id: String::new(),
    }
}

// =============================================================================
// Fixtures
// =============================================================================

/// Records every `publish_event`, together with whether the lifecycle token was
/// already cancelled when it landed — which is how the ordering test tells a
/// real flush apart from one that returned instantly on a dead drainer.
#[derive(Default)]
struct RecordingBackend {
    delivered: Mutex<Vec<String>>,
    publish_attempts: AtomicUsize,
    published_after_cancel: AtomicUsize,
    fail_first: AtomicUsize,
    lifecycle: Mutex<Option<CancellationToken>>,
}

impl RecordingBackend {
    fn fail_first_publish(&self) {
        self.fail_first.store(1, Ordering::SeqCst);
    }

    fn watch_lifecycle(&self, cancel: CancellationToken) {
        *self.lifecycle.lock().unwrap() = Some(cancel);
    }

    fn publish_attempts(&self) -> usize {
        self.publish_attempts.load(Ordering::SeqCst)
    }

    fn delivered(&self) -> Vec<String> {
        self.delivered.lock().unwrap().clone()
    }

    fn all_publishes_saw_a_live_lifecycle(&self) -> bool {
        self.published_after_cancel.load(Ordering::SeqCst) == 0
    }
}

#[async_trait]
impl BackendClient for RecordingBackend {
    async fn publish_event(&self, req: PublishEventRequest) -> Result<(), tonic::Status> {
        self.publish_attempts.fetch_add(1, Ordering::SeqCst);
        if let Some(cancel) = self.lifecycle.lock().unwrap().as_ref() {
            if cancel.is_cancelled() {
                self.published_after_cancel.fetch_add(1, Ordering::SeqCst);
            }
        }
        if self.fail_first.swap(0, Ordering::SeqCst) > 0 {
            return Err(tonic::Status::unavailable("transient"));
        }
        self.delivered.lock().unwrap().push(req.sub_type);
        Ok(())
    }

    async fn get_pending_messages(
        &self,
        _: &str,
    ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
        Ok(vec![])
    }

    async fn acknowledge_messages(
        &self,
        _: &str,
        _: Vec<String>,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn get_credentials(&self, _: &str) -> Result<GetCredentialsResponse, tonic::Status> {
        Ok(GetCredentialsResponse::default())
    }

    async fn update_credential(&self, _: &str, _: &str, _: &str) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn get_system_prompt(&self, _: &str) -> Result<String, tonic::Status> {
        Ok(String::new())
    }

    async fn download_upload(
        &self,
        _: &str,
        _: &str,
    ) -> Result<DownloadUploadResponse, tonic::Status> {
        Err(tonic::Status::not_found("no uploads in this fixture"))
    }

    async fn recover_interrupted_run(
        &self,
        _: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
        Ok(None)
    }

    async fn upload_transcript(&self, _: &str, _: Vec<u8>) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn download_transcript(&self, _: &str) -> Result<Vec<u8>, tonic::Status> {
        Err(tonic::Status::not_found("no transcript in this fixture"))
    }
}

/// Every directory points into `dir`, which the caller keeps empty: `build_tar`
/// then finds no session state, so `transcript_sync::flush_on_shutdown` returns
/// immediately instead of spending its 12s budget. `workspace_sync_enabled` is
/// false for the same reason.
fn test_config(dir: &std::path::Path) -> Config {
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    Config {
        port: 0,
        grpc_port: 0,
        task_id: "test-task".to_string(),
        sandbox_id: "test-task".to_string(),
        agent_type: Some("claude-code".to_string()),
        workspace_dir: path("workspace"),
        repo_dir: path("workspace"),
        data_dir: path("data"),
        runtime_dir: path("run"),
        upload_dir: path("upload"),
        home_dir: path("home"),
        codex_home_dir: path("codex-home"),
        log_level: "info".to_string(),
        backend_grpc_url: "http://localhost:50051".to_string(),
        backend_api_url: None,
        user_auth_token: None,
        bootstrap_token: None,
        agent_config: moatless_sandbox::config::AgentConfig::default(),
        claude_cli_path: path("bin/claude"),
        codex_cli_path: path("bin/codex"),
        proxy: moatless_sandbox::config::ProxyConfig::default(),
        servers: vec![],
        claude_config_dir: path("claude-config"),
        git_clone_url: None,
        git_branch: None,
        git_provider: "github".to_string(),
        repos: Vec::new(),
        repo_config_raw: None,
        setup_commands: vec![],
        plugins: vec![],
        frontend_url: None,
        use_tmux: false,
        command_default_timeout_secs: moatless_sandbox::config::DEFAULT_COMMAND_TIMEOUT_SECS,
        default_model: None,
        default_codex_model: None,
        openai_base_url: None,
        permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
        workspace_sync_enabled: false,
        workspace_snapshot_format: WorkspaceSnapshotFormat::Mirror,
        workspace_restore_if_empty: false,
        headless: false,
    }
}

/// A `SandboxState` whose agent lifecycle task is real, so
/// `agent_handle.is_closed()` answers honestly for the liveness assertions.
/// `dir` must outlive the state.
fn test_state(backend: Arc<dyn BackendClient>, dir: &tempfile::TempDir) -> SandboxState {
    let lifecycle_cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(
        Arc::clone(&backend),
        "test-task".to_string(),
        lifecycle_cancel.clone(),
    );
    let events = GrpcEventPublisher::new(outbox);
    let config = Arc::new(test_config(dir.path()));
    let agent_state = Arc::new(AgentState::new());
    let credentials = Arc::new(RwLock::new(HashMap::new()));
    let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new())));
    let credential_dispatcher = Arc::new(Dispatcher::new());
    let credential_failures = CredentialFailureDetector::default();
    let system_prompt_cache: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
    let heartbeat = Heartbeat::new();

    let commands = Arc::new(moatless_sandbox::commands::CommandRegistry::new(
        std::time::Duration::from_mins(1),
        std::env::temp_dir().join("moat-cmd-it"),
    ));

    let agent_handle = spawn_agent(
        Arc::clone(&config),
        Arc::clone(&agent_state),
        events.clone(),
        Arc::clone(&credentials),
        Arc::clone(&backend),
        Arc::clone(&scheduled_jobs),
        Arc::clone(&commands),
        credential_failures.clone(),
        Arc::clone(&credential_dispatcher),
        Arc::clone(&system_prompt_cache),
        heartbeat.clone(),
        moatless_sandbox::workspace_sync::WorkspaceSyncTracker::new(),
    );
    let task_context = Arc::new(TaskContext::new(
        config.task_id.clone(),
        config.workspace_dir.clone(),
        config.repo_dir.clone(),
        moatless_sandbox::state::CheckoutMode::Shared,
        agent_state,
        agent_handle,
        scheduled_jobs,
        commands,
        credential_failures,
        system_prompt_cache,
    ));
    let task_contexts = Arc::new(RwLock::new(HashMap::from([(
        task_context.task_id.clone(),
        task_context,
    )])));
    let terminals = moatless_sandbox::terminal::TerminalManager::new(config.workspace_dir.clone());

    SandboxState {
        workspace_sync: Default::default(),
        config,
        events,
        task_contexts,
        task_context_registration: Arc::new(tokio::sync::Mutex::new(())),
        heartbeat,
        backend_client: backend,
        init_phase: Arc::new(RwLock::new(InitPhase::Ready)),
        credentials,
        lifecycle_cancel,
        credential_dispatcher,
        codex_watcher_handle: Arc::new(RwLock::new(None)),
        supervisor_health: Default::default(),
        terminals,
    }
}
