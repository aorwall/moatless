//! Shared fixtures for the sandbox-replacement lifecycle tests: an in-memory
//! backend, a `SandboxState` whose agent is a real coordinator driving a
//! recording scripted harness, and small builders.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingMessage,
    PublishEventRequest,
};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::agent::agent_state::AgentState;
use crate::agent::coordinator::{Coordinator, CoordinatorService};
use crate::agent::handle::AgentHandle;
use crate::agent::harness::{Harness, Input, Script, SessionId, Step};
use crate::agent::heartbeat::Heartbeat;
use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
use crate::backend_client::BackendClient;
use crate::config::test_support::minimal_config;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::Dispatcher;
use crate::events::GrpcEventPublisher;
use crate::init::InitPhase;
use crate::outbox::EventOutbox;
use crate::state::SandboxState;

/// A shared sink of the full `Input`s the harness accepted, in submit order.
pub(super) type InputRecorder = Arc<Mutex<Vec<Input>>>;

/// A backend whose pending-message result and interrupted-Run payload are fixed
/// per test. `acknowledge_messages` clears the acked message so a later probe
/// sees it gone — modelling the real ack that the startup race hinges on.
pub(super) struct ReplayBackend {
    /// `Ok(list)` for a normal probe; `Err(())` to model a backend blip. Behind
    /// a `Mutex` only because `acknowledge_messages` prunes it.
    pending: Mutex<Result<Vec<PendingMessage>, ()>>,
    /// The interrupted turn `recover_interrupted_run` returns (immutable).
    interrupted: Option<InterruptedTurn>,
    /// The session-state archive `download_transcript` serves (immutable).
    download_tar: Vec<u8>,
    get_pending_calls: AtomicUsize,
    recover_calls: AtomicUsize,
}

impl ReplayBackend {
    pub(super) fn new(
        pending: Result<Vec<PendingMessage>, ()>,
        interrupted: Option<InterruptedTurn>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(pending),
            interrupted,
            download_tar: Vec::new(),
            get_pending_calls: AtomicUsize::new(0),
            recover_calls: AtomicUsize::new(0),
        })
    }

    /// A backend that serves `tar` from `download_transcript`, with no pending
    /// messages and no interrupted run — for the restore tests.
    pub(super) fn with_archive(tar: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(Ok(Vec::new())),
            interrupted: None,
            download_tar: tar,
            get_pending_calls: AtomicUsize::new(0),
            recover_calls: AtomicUsize::new(0),
        })
    }

    pub(super) fn get_pending_calls(&self) -> usize {
        self.get_pending_calls.load(Ordering::SeqCst)
    }

    pub(super) fn recover_calls(&self) -> usize {
        self.recover_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BackendClient for ReplayBackend {
    async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn get_pending_messages(
        &self,
        _task_id: &str,
    ) -> Result<Vec<PendingMessage>, tonic::Status> {
        self.get_pending_calls.fetch_add(1, Ordering::SeqCst);
        match &*self.pending.lock().await {
            Ok(msgs) => Ok(msgs.clone()),
            Err(()) => Err(tonic::Status::unavailable("simulated probe failure")),
        }
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
    ) -> Result<(), tonic::Status> {
        // Prune acked messages so a subsequent probe no longer returns them —
        // exactly what lets the startup race manifest if ordering is wrong.
        if let Ok(msgs) = &mut *self.pending.lock().await {
            msgs.retain(|m| {
                m.uuid
                    .as_ref()
                    .is_none_or(|u| !message_uuids.contains(u))
            });
        }
        Ok(())
    }

    async fn get_credentials(
        &self,
        _task_id: &str,
    ) -> Result<GetCredentialsResponse, tonic::Status> {
        Ok(GetCredentialsResponse {
            credentials: HashMap::new(),
            server_env: HashMap::new(),
            ssh_keys: HashMap::new(),
        })
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
        Err(tonic::Status::unimplemented("not used"))
    }

    async fn recover_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
        self.recover_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.interrupted.clone())
    }

    async fn upload_transcript(
        &self,
        _task_id: &str,
        _tar_data: Vec<u8>,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, tonic::Status> {
        Ok(self.download_tar.clone())
    }
}

/// Assemble a `SandboxState` from its parts. `agent_handle` is either a real
/// recording coordinator (lifecycle tests) or a bare channel (restore tests).
fn build_state(
    config: Arc<Config>,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
    agent: Arc<AgentState>,
    agent_handle: AgentHandle,
    events: GrpcEventPublisher,
) -> SandboxState {
    SandboxState {
        config,
        events,
        agent,
        agent_handle,
        heartbeat: Heartbeat::new(),
        scheduled_jobs: Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
        backend_client: backend,
        init_phase: Arc::new(RwLock::new(phase)),
        credentials: Arc::new(RwLock::new(HashMap::new())),
        lifecycle_cancel: CancellationToken::new(),
        credential_dispatcher: Arc::new(Dispatcher::new()),
        credential_failures: CredentialFailureDetector::default(),
        codex_watcher_handle: Arc::new(RwLock::new(None)),
        system_prompt_cache: Arc::new(RwLock::new(None)),
    }
}

/// A fire-and-forget event publisher over `backend` (its outbox lives for the
/// test's scope via a fresh cancel token).
fn publisher_for(config: &Config, backend: Arc<dyn BackendClient>) -> GrpcEventPublisher {
    let outbox = EventOutbox::spawn(backend, config.task_id.clone(), CancellationToken::new());
    GrpcEventPublisher::new(outbox)
}

/// Spawn a real `CoordinatorService` over a recording scripted harness, consuming
/// the command channel behind `agent_handle`. Every input the lifecycle path
/// drives — `Send` or `Recover` — flows through the production coordinator into
/// the harness and is recorded, so a test reads back what the harness accepted.
fn spawn_recording_agent(
    config: &Arc<Config>,
    backend: Arc<dyn BackendClient>,
    agent: Arc<AgentState>,
    events: GrpcEventPublisher,
    recorder: InputRecorder,
) -> AgentHandle {
    let (agent_handle, command_rx) = AgentHandle::test_channel();

    // Each execution reports a session, accepts its input, and completes — enough
    // for the coordinator to mint a Run and resolve the send ack (which drives
    // the backend acknowledgement the race hinges on).
    let script = Script::new(vec![
        Step::SessionReady(SessionId("recorded-session".to_string())),
        Step::AcceptInput,
        Step::Completed,
    ]);
    let harness = Harness::scripted_recording(script, recorder);
    let coordinator = Coordinator::new(harness, events, config.task_id.clone());
    let service = CoordinatorService::new(
        coordinator,
        Arc::clone(&agent),
        Arc::clone(config),
        Arc::new(RwLock::new(HashMap::new())),
        Arc::clone(&backend),
        Arc::new(RwLock::new(None)),
        Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
        CredentialFailureDetector::default(),
        Arc::new(Dispatcher::new()),
    );
    tokio::spawn(service.run(command_rx, Heartbeat::new()));
    agent_handle
}

/// A `SandboxState` whose agent is a real recording coordinator. `agent_type`
/// fixes the harness family; `phase` sets the init phase the readiness gate
/// reads. Returns the state and the recorder of accepted `Input`s.
pub(super) fn recording_state(
    agent_type: Option<&str>,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
) -> (SandboxState, InputRecorder) {
    let mut config = minimal_config();
    config.agent_type = agent_type.map(ToString::to_string);
    let config = Arc::new(config);

    let agent = Arc::new(AgentState::new());
    let events = publisher_for(&config, Arc::clone(&backend));
    let recorder: InputRecorder = Arc::new(Mutex::new(Vec::new()));
    let agent_handle = spawn_recording_agent(
        &config,
        Arc::clone(&backend),
        Arc::clone(&agent),
        events.clone(),
        Arc::clone(&recorder),
    );

    let state = build_state(config, phase, backend, agent, agent_handle, events);
    (state, recorder)
}

/// A `SandboxState` over an explicit `config` with a bare (undriven) agent
/// channel — for restore tests that call `download_on_start` and never touch
/// the agent.
pub(super) fn restore_state(
    config: Arc<Config>,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
) -> SandboxState {
    let agent = Arc::new(AgentState::new());
    let events = publisher_for(&config, Arc::clone(&backend));
    let (agent_handle, _command_rx) = AgentHandle::test_channel();
    build_state(config, phase, backend, agent, agent_handle, events)
}

// ── Builders ─────────────────────────────────────────────────────────────────

pub(super) fn text_message(text: &str, uuid: &str) -> PendingMessage {
    PendingMessage {
        content: vec![sandbox_proto::ContentBlock {
            block: Some(sandbox_proto::content_block::Block::Text(
                sandbox_proto::TextBlock {
                    text: text.to_string(),
                },
            )),
        }],
        uuid: Some(uuid.to_string()),
        ..Default::default()
    }
}

pub(super) fn interrupted_turn(
    session: &str,
    prompt: &str,
    agent_type: Option<&str>,
) -> InterruptedTurn {
    InterruptedTurn {
        session_id: session.to_string(),
        turn_number: 1,
        prompt_text: prompt.to_string(),
        agent_type: agent_type.map(ToString::to_string),
        agent_mode: None,
    }
}

/// A config whose Claude/Codex/workspace dirs live under an isolated tmp dir, so
/// restore lands in a place the test can inspect without touching shared paths.
pub(super) fn tmp_config(agent_type: &str, tmp: &tempfile::TempDir) -> Config {
    let mut config = minimal_config();
    config.agent_type = Some(agent_type.to_string());
    config.workspace_dir = tmp.path().join("workspace").display().to_string();
    config.repo_dir = config.workspace_dir.clone();
    config.claude_config_dir = tmp.path().join("claude-config").display().to_string();
    config.codex_home_dir = tmp.path().join("codex-home").display().to_string();
    config
}

/// Build the combined archive exactly as the backend serves it: Claude entries
/// under `claude/`, Codex rollouts under `codex/sessions/`.
pub(super) async fn combined_archive() -> Vec<u8> {
    let staging = tempfile::TempDir::new().expect("tmp");
    let claude_src = staging.path().join("claude");
    let codex_src = staging.path().join("codex").join("sessions").join("2026");
    tokio::fs::create_dir_all(&claude_src).await.unwrap();
    tokio::fs::create_dir_all(&codex_src).await.unwrap();
    tokio::fs::write(claude_src.join("session.jsonl"), b"{\"turn\":1}\n")
        .await
        .unwrap();
    tokio::fs::write(codex_src.join("rollout-xyz.jsonl"), b"{\"thread\":\"xyz\"}\n")
        .await
        .unwrap();
    let root = staging.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut b = tar::Builder::new(Vec::new());
        b.append_dir_all("claude", root.join("claude")).unwrap();
        b.append_dir_all("codex/sessions", root.join("codex").join("sessions"))
            .unwrap();
        b.into_inner().unwrap()
    })
    .await
    .unwrap()
}

/// Poll the recorder until it holds at least `n` inputs or the deadline passes,
/// then return a snapshot. Bounded so a wiring regression fails fast instead of
/// hanging.
pub(super) async fn recorded_inputs(recorder: &InputRecorder, n: usize) -> Vec<Input> {
    let deadline = Duration::from_secs(5);
    let step = Duration::from_millis(5);
    let mut waited = Duration::ZERO;
    loop {
        {
            let inputs = recorder.lock().await;
            if inputs.len() >= n {
                return inputs.clone();
            }
        }
        if waited >= deadline {
            return recorder.lock().await.clone();
        }
        tokio::time::sleep(step).await;
        waited += step;
    }
}

/// Give the runtime a beat for any in-flight command to settle, then snapshot the
/// recorder — used to assert that *nothing* was accepted.
pub(super) async fn settled_inputs(recorder: &InputRecorder) -> Vec<Input> {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    recorder.lock().await.clone()
}
