//! Shared fixtures for the sandbox-replacement lifecycle tests: an in-memory
//! backend, a `SandboxState` whose agent is a real coordinator driving a
//! recording scripted harness, and small builders.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sandbox_proto::{InterruptedTurn, PendingAgentDelivery};
use tokio::sync::{Mutex, RwLock, oneshot};
use tokio_util::sync::CancellationToken;

use crate::agent::agent_state::AgentState;
use crate::agent::coordinator::{Coordinator, CoordinatorService};
use crate::agent::handle::AgentHandle;
use crate::agent::harness::{HarnessRuntime, Input, Script, SessionId, Step};
use crate::agent::heartbeat::Heartbeat;
use crate::agent::prompt::{PendingPrompt, PromptResponse, PromptToolName};
use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::config::test_support::minimal_config;
use crate::credentials::Dispatcher;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::events::GrpcEventPublisher;
use crate::init::{InitPhase, SetupGate};
use crate::outbox::EventOutbox;
use crate::state::{SandboxState, TaskContext};

/// A shared sink of the full `Input`s the harness accepted, in submit order.
pub(super) type InputRecorder = Arc<Mutex<Vec<Input>>>;

/// Assemble a `SandboxState` from its parts. `agent_handle` is either a real
/// recording coordinator (lifecycle tests) or a bare channel (restore tests).
fn build_state(
    config: Arc<Config>,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
    agent: Arc<AgentState>,
    agent_handle: AgentHandle,
    events: GrpcEventPublisher,
    setup_gate: SetupGate,
) -> SandboxState {
    let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new())));
    let task_context = Arc::new(TaskContext::new(
        config.task_id.clone(),
        config.workspace_dir.clone(),
        config.repo_dir.clone(),
        crate::state::CheckoutMode::Shared,
        config.agent_profile.clone(),
        agent,
        agent_handle,
        scheduled_jobs,
        Arc::new(crate::commands::CommandRegistry::for_test()),
        CredentialFailureDetector::default(),
        Arc::new(RwLock::new(None)),
    ));
    let terminals = crate::terminal::TerminalManager::with_repo_dir(
        config.workspace_dir.clone(),
        config.repo_dir.clone(),
    );
    SandboxState {
        config,
        events,
        task_contexts: Arc::new(RwLock::new(HashMap::from([(
            task_context.task_id.clone(),
            task_context,
        )]))),
        task_context_registration: Arc::new(tokio::sync::Mutex::new(())),
        heartbeat: Heartbeat::new(),
        backend_client: backend,
        workspace_sync: Default::default(),
        writer_admission: Default::default(),
        init_phase: Arc::new(RwLock::new(phase)),
        setup_gate,
        workspace_origin: Default::default(),
        stop_cause: Default::default(),
        git_credentials: Default::default(),
        credentials: Arc::new(RwLock::new(HashMap::new())),
        lifecycle_cancel: CancellationToken::new(),
        credential_dispatcher: Arc::new(Dispatcher::new()),
        codex_watcher_handle: Arc::new(RwLock::new(None)),
        codex_via_gateway: Default::default(),
        supervisor_health: Default::default(),
        terminals,
    }
}

/// A fire-and-forget event publisher over `backend` (its outbox lives for the
/// test's scope via a fresh cancel token).
fn publisher_for(config: &Config, backend: Arc<dyn BackendClient>) -> GrpcEventPublisher {
    let (outbox, _drainer) =
        EventOutbox::spawn(backend, config.task_id.clone(), CancellationToken::new());
    GrpcEventPublisher::new(outbox)
}

/// Spawn a real `CoordinatorService` over a recording scripted harness,
/// consuming the command channel behind `agent_handle`.
///
/// Every `Send` or `Recover` flows through the production coordinator into the
/// harness and is recorded, so a test reads back what was accepted.
fn spawn_recording_agent(
    config: &Arc<Config>,
    backend: Arc<dyn BackendClient>,
    agent: Arc<AgentState>,
    events: GrpcEventPublisher,
    recorder: InputRecorder,
    setup_gate: SetupGate,
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
    let harness = HarnessRuntime::scripted_recording(script, recorder);
    let coordinator = Coordinator::new(harness, events, config.task_id.clone());
    let service = CoordinatorService::new(
        coordinator,
        Arc::clone(&agent),
        Arc::clone(config),
        Arc::new(RwLock::new(HashMap::new())),
        Arc::clone(&backend),
        Arc::new(RwLock::new(None)),
        Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
        Arc::new(crate::commands::CommandRegistry::for_test()),
        CredentialFailureDetector::default(),
        Arc::new(Dispatcher::new()),
        Default::default(),
        crate::workspace_sync::WorkspaceSyncTracker::new(),
    );
    tokio::spawn(service.run_after(setup_gate, command_rx, Heartbeat::new()));
    agent_handle
}

/// A `SandboxState` whose agent is a real recording coordinator. `agent_profile`
/// fixes the harness family; `phase` sets the init phase the readiness gate
/// reads. Returns the state and the recorder of accepted `Input`s.
pub(super) fn recording_state(
    agent_profile: Option<&str>,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
) -> (SandboxState, InputRecorder) {
    let mut config = minimal_config();
    config.agent_profile = agent_profile.map(ToString::to_string);
    recording_state_with_config(config, phase, backend)
}

/// Like [`recording_state`] but behind `setup_gate`, as a pod is until its
/// workspace setup has run.
pub(super) fn recording_state_behind(
    setup_gate: SetupGate,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
) -> (SandboxState, InputRecorder) {
    let mut config = minimal_config();
    config.agent_profile = Some("claude-code".to_string());
    recording_state_gated(config, phase, backend, setup_gate)
}

/// Like [`recording_state`] but over a caller-supplied `config` — used by the
/// restore-then-replay lifecycle test, whose config points restore at
/// inspectable tmp dirs.
pub(super) fn recording_state_with_config(
    config: Config,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
) -> (SandboxState, InputRecorder) {
    recording_state_gated(config, phase, backend, SetupGate::opened())
}

/// Like [`recording_state_with_config`] but behind `setup_gate`.
pub(super) fn recording_state_gated(
    config: Config,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
    setup_gate: SetupGate,
) -> (SandboxState, InputRecorder) {
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
        setup_gate.clone(),
    );

    let state = build_state(
        config,
        phase,
        backend,
        agent,
        agent_handle,
        events,
        setup_gate,
    );
    (state, recorder)
}

/// Register `task_id` as a second Member of `state`, sharing its checkout and
/// driving its own recording coordinator under its own task id. Returns that
/// Member's recorder, so a test reads back which Member each `Input` reached.
pub(super) async fn add_recording_member(state: &SandboxState, task_id: &str) -> InputRecorder {
    let member_config = Arc::new(Config {
        task_id: task_id.to_string(),
        ..(*state.config).clone()
    });
    let agent = Arc::new(AgentState::new());
    let recorder: InputRecorder = Arc::new(Mutex::new(Vec::new()));
    let agent_handle = spawn_recording_agent(
        &member_config,
        Arc::clone(&state.backend_client),
        Arc::clone(&agent),
        state.events.clone(),
        Arc::clone(&recorder),
        state.setup_gate.clone(),
    );
    let context = Arc::new(TaskContext::new(
        task_id.to_string(),
        state.config.workspace_dir.clone(),
        state.config.repo_dir.clone(),
        crate::state::CheckoutMode::Shared,
        state.config.agent_profile.clone(),
        agent,
        agent_handle,
        Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
        Arc::new(crate::commands::CommandRegistry::for_test()),
        CredentialFailureDetector::default(),
        Arc::new(RwLock::new(None)),
    ));
    state
        .task_contexts
        .write()
        .await
        .insert(task_id.to_string(), context);
    recorder
}

/// A `SandboxState` over an explicit `config` with a bare (undriven) agent
/// channel — for a restore test that calls `download_on_start` and never
/// touches the agent, and for a dispatch test that needs the send's ack
/// dropped.
pub(super) fn restore_state(
    config: Arc<Config>,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
) -> SandboxState {
    let agent = Arc::new(AgentState::new());
    let events = publisher_for(&config, Arc::clone(&backend));
    let (agent_handle, _command_rx) = AgentHandle::test_channel();
    build_state(
        config,
        phase,
        backend,
        agent,
        agent_handle,
        events,
        SetupGate::opened(),
    )
}

// ── Builders ─────────────────────────────────────────────────────────────────

pub(super) fn text_message(text: &str, uuid: &str) -> PendingAgentDelivery {
    PendingAgentDelivery {
        message_uuids: vec![uuid.to_string()],
        prompt_text: text.to_string(),
        ..Default::default()
    }
}

pub(super) fn interrupted_turn(
    session: &str,
    prompt: &str,
    agent_profile: Option<&str>,
) -> InterruptedTurn {
    InterruptedTurn {
        session_id: session.to_string(),
        turn_number: 1,
        prompt_text: prompt.to_string(),
        agent_profile: agent_profile.map(ToString::to_string),
        agent_mode: None,
        effort: None,
        run_id: None,
        model: None,
    }
}

/// A config whose Claude/Codex/workspace dirs live under an isolated tmp dir,
/// so restore lands in a place the test can inspect without touching shared
/// paths.
pub(super) fn tmp_config(agent_profile: &str, tmp: &tempfile::TempDir) -> Config {
    let mut config = minimal_config();
    config.agent_profile = Some(agent_profile.to_string());
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
    tokio::fs::write(
        codex_src.join("rollout-xyz.jsonl"),
        b"{\"thread\":\"xyz\"}\n",
    )
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

/// Give the runtime a beat for any in-flight command to settle, then snapshot
/// the recorder — used to assert that *nothing* was accepted.
pub(super) async fn settled_inputs(recorder: &InputRecorder) -> Vec<Input> {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    recorder.lock().await.clone()
}

/// Park an `ExitPlanMode` on `context`, under the run identity
/// `resolve_pending_prompt` needs to answer one. Returns the receiver the
/// parked call waits on, so a test reads back whether it was answered.
pub(super) async fn park_plan_prompt(
    context: &TaskContext,
    tool_use_id: &str,
) -> oneshot::Receiver<PromptResponse> {
    context.agent.set_session_id("sess-1".to_string()).await;
    context.agent.set_run_id("run-1".to_string()).await;
    let (sender, receiver) = oneshot::channel();
    assert!(
        context
            .agent
            .set_pending_prompt(PendingPrompt {
                tool_name: PromptToolName::ExitPlanMode,
                tool_use_id: tool_use_id.to_string(),
                input: serde_json::json!({}),
                sender,
            })
            .await,
        "the run already holds a parked prompt"
    );
    receiver
}
