//! Sandbox-replacement lifecycle tests.
//!
//! When a pod is replaced, `run_init` restores Claude/Codex Session state to
//! disk (before Ready), then either replays the interrupted Run or lets a newer
//! pending Message supersede it. Two ordering invariants must hold and are proven
//! here at the highest stable in-crate seam — the `AgentCommand` channel that the
//! lifecycle owner (`CoordinatorService`) drains FIFO into the harness, so the
//! order a command is enqueued is the order the harness accepts input:
//!
//! 1. **Readiness gate** — a backend notification that races init (the gRPC
//!    server accepts calls from pod start) must not drive the harness before
//!    restoration. `dispatch_pending_messages` no-ops until Ready.
//! 2. **Interrupted-Run precedence** — with no pending Message the interrupted
//!    Run is replayed (`Recover` carrying the restored session id); with a
//!    pending Message the replay is skipped and the pending text is what runs
//!    (`Send`). A pending-probe error fails closed (skip the replay).
//!
//! These run for both selected harness families (claude-code and codex): a
//! replacement pod reconstructs the family fixed by `config.agent_type` (proven
//! by `build_harness` in `agent::handle`) and drives the restored session for it.
//! The resume *resolution* per family is proven by the `build_resume` unit tests;
//! here we prove the replacement path enqueues the restored session for each.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingMessage,
    PublishEventRequest,
};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::{recover_interrupted_run_unless_pending, InitPhase, WorkspaceOrigin};
use crate::agent::agent_state::AgentState;
use crate::agent::dispatcher::dispatch_pending_messages;
use crate::agent::handle::{AgentCommand, AgentHandle};
use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
use crate::backend_client::BackendClient;
use crate::config::test_support::minimal_config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::Dispatcher;
use crate::events::GrpcEventPublisher;
use crate::state::SandboxState;

/// A backend whose pending-message result and interrupted-Run payload are fixed
/// per test, with call counters for the two probes the lifecycle path makes.
struct ReplayBackend {
    /// `Ok(list)` for a normal probe; `Err` to model a backend blip.
    pending: Mutex<Result<Vec<PendingMessage>, ()>>,
    interrupted: Option<InterruptedTurn>,
    /// Session-state archive `download_transcript` returns (empty = none stored).
    download_tar: Mutex<Vec<u8>>,
    get_pending_calls: AtomicUsize,
    recover_calls: AtomicUsize,
}

impl ReplayBackend {
    fn new(
        pending: Result<Vec<PendingMessage>, ()>,
        interrupted: Option<InterruptedTurn>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(pending),
            interrupted,
            download_tar: Mutex::new(Vec::new()),
            get_pending_calls: AtomicUsize::new(0),
            recover_calls: AtomicUsize::new(0),
        })
    }

    fn with_download_tar(self: &Arc<Self>, tar: Vec<u8>) {
        *self.download_tar.lock().expect("tar lock") = tar;
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
        match &*self.pending.lock().expect("pending lock") {
            Ok(msgs) => Ok(msgs.clone()),
            Err(()) => Err(tonic::Status::unavailable("simulated probe failure")),
        }
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
    ) -> Result<(), tonic::Status> {
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
        Ok(self.download_tar.lock().expect("tar lock").clone())
    }
}

/// Build a `SandboxState` whose `AgentHandle` is backed by an observable command
/// channel (no real actor/coordinator), so a test reads back exactly which
/// `AgentCommand`s the lifecycle path enqueues. `agent_type` fixes the harness
/// family; `phase` sets the init phase the readiness gate reads.
fn replacement_state(
    agent_type: Option<&str>,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
) -> (SandboxState, UnboundedReceiver<AgentCommand>) {
    let mut config = minimal_config();
    config.agent_type = agent_type.map(ToString::to_string);
    state_from_config(Arc::new(config), phase, backend)
}

fn state_from_config(
    config: Arc<crate::config::Config>,
    phase: InitPhase,
    backend: Arc<dyn BackendClient>,
) -> (SandboxState, UnboundedReceiver<AgentCommand>) {
    let outbox = crate::outbox::EventOutbox::spawn(
        Arc::clone(&backend),
        config.task_id.clone(),
        CancellationToken::new(),
    );
    let events = GrpcEventPublisher::new(outbox);

    let late_injector = Arc::new(LateInjector::new());
    let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::clone(&late_injector) as _));

    let (agent_handle, command_rx) = AgentHandle::test_channel();

    let state = SandboxState {
        config,
        events,
        agent: Arc::new(AgentState::new()),
        agent_handle,
        heartbeat: crate::agent::heartbeat::Heartbeat::new(),
        scheduled_jobs,
        backend_client: backend,
        init_phase: Arc::new(RwLock::new(phase)),
        credentials: Arc::new(RwLock::new(HashMap::new())),
        lifecycle_cancel: CancellationToken::new(),
        credential_dispatcher: Arc::new(Dispatcher::new()),
        credential_failures: CredentialFailureDetector::default(),
        codex_watcher_handle: Arc::new(RwLock::new(None)),
        system_prompt_cache: Arc::new(RwLock::new(None)),
    };
    (state, command_rx)
}

fn text_message(text: &str, uuid: &str) -> PendingMessage {
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

fn interrupted_turn(session: &str, prompt: &str, agent_type: Option<&str>) -> InterruptedTurn {
    InterruptedTurn {
        session_id: session.to_string(),
        turn_number: 1,
        prompt_text: prompt.to_string(),
        agent_type: agent_type.map(ToString::to_string),
        agent_mode: None,
    }
}

/// Drain every queued command without blocking.
fn drain(rx: &mut UnboundedReceiver<AgentCommand>) -> Vec<AgentCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// Variant names for failure messages — `AgentCommand` is intentionally not
/// `Debug` (it holds oneshot senders), so describe the shape instead.
fn kinds(cmds: &[AgentCommand]) -> Vec<&'static str> {
    cmds.iter()
        .map(|c| match c {
            AgentCommand::Send { .. } => "send",
            AgentCommand::Recover { .. } => "recover",
            AgentCommand::Stop { .. } => "stop",
        })
        .collect()
}

// ── Readiness gate ───────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_is_skipped_before_ready() {
    // A notification that races init must not fetch messages or drive the
    // harness: the harness may still be resuming un-restored Session state.
    let backend = ReplayBackend::new(Ok(vec![text_message("hello", "uuid-1")]), None);
    let (state, mut rx) = replacement_state(Some("claude-code"), InitPhase::Starting, backend.clone());

    dispatch_pending_messages(state).await;

    assert!(
        drain(&mut rx).is_empty(),
        "no command may reach the harness before Ready"
    );
    assert_eq!(
        backend.get_pending_calls.load(Ordering::SeqCst),
        0,
        "the gate short-circuits before the backend is even probed"
    );
}

#[tokio::test]
async fn dispatch_drives_the_pending_message_after_ready() {
    let backend = ReplayBackend::new(Ok(vec![text_message("hello", "uuid-1")]), None);
    let (state, mut rx) = replacement_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    dispatch_pending_messages(state).await;

    let cmds = drain(&mut rx);
    assert!(
        matches!(cmds.as_slice(), [AgentCommand::Send { text, .. }] if text == "hello"),
        "after Ready the pending message is driven, got {:?}",
        kinds(&cmds)
    );
    assert!(backend.get_pending_calls.load(Ordering::SeqCst) >= 1);
}

// ── Interrupted-Run precedence ───────────────────────────────────────────────

#[tokio::test]
async fn no_pending_message_replays_the_interrupted_run_for_each_family() {
    // For each selected family, the restored Run is replayed: a `Recover`
    // carrying the restored session id and prompt is what the harness accepts.
    for family in ["claude-code", "codex"] {
        let backend = ReplayBackend::new(
            Ok(vec![]),
            Some(interrupted_turn("restored-sess", "prior work", Some(family))),
        );
        let (state, mut rx) =
            replacement_state(Some(family), InitPhase::Ready, backend.clone());

        recover_interrupted_run_unless_pending(&state, WorkspaceOrigin::Persisted).await;

        let cmds = drain(&mut rx);
        assert!(
            matches!(
                cmds.as_slice(),
                [AgentCommand::Recover { resume_session_id, text, .. }]
                    if resume_session_id == "restored-sess" && text == "prior work"
            ),
            "family {family}: expected a Recover for the restored session, got {:?}",
            kinds(&cmds)
        );
        assert_eq!(
            backend.recover_calls.load(Ordering::SeqCst),
            1,
            "family {family}: the interrupted run is probed exactly once"
        );
    }
}

#[tokio::test]
async fn pending_message_supersedes_the_interrupted_run() {
    // The interrupted Run exists, but a newer pending Message is present. The
    // replay must be skipped, and the pending Message is what actually runs —
    // proving which input the harness accepts first when both are available.
    let backend = ReplayBackend::new(
        Ok(vec![text_message("new instruction", "uuid-new")]),
        Some(interrupted_turn("restored-sess", "prior work", Some("claude-code"))),
    );
    let (state, mut rx) = replacement_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    recover_interrupted_run_unless_pending(&state, WorkspaceOrigin::Persisted).await;
    assert!(
        drain(&mut rx).is_empty(),
        "a pending message must suppress the interrupted-run replay"
    );
    assert_eq!(
        backend.recover_calls.load(Ordering::SeqCst),
        0,
        "the interrupted run must not even be probed when a message is pending"
    );

    // The authoritative dispatch then drives the pending message, not the Run.
    dispatch_pending_messages(state).await;
    let cmds = drain(&mut rx);
    assert!(
        matches!(cmds.as_slice(), [AgentCommand::Send { text, .. }] if text == "new instruction"),
        "the pending message is what runs, got {:?}",
        kinds(&cmds)
    );
}

#[tokio::test]
async fn pending_probe_error_fails_closed_and_skips_replay() {
    // An unknown probe result must NOT replay the stale Run: doing so could
    // resume it and then race a newer Message once the backend recovers.
    let backend = ReplayBackend::new(
        Err(()),
        Some(interrupted_turn("restored-sess", "prior work", Some("claude-code"))),
    );
    let (state, mut rx) = replacement_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    recover_interrupted_run_unless_pending(&state, WorkspaceOrigin::Persisted).await;

    assert!(
        drain(&mut rx).is_empty(),
        "a failed probe must fail closed — no interrupted-run replay"
    );
    assert_eq!(
        backend.recover_calls.load(Ordering::SeqCst),
        0,
        "a failed probe must not drive interrupted-run recovery"
    );
}

// ── Session-state restore (runs before Ready) ────────────────────────────────

/// A config whose Claude/Codex/workspace dirs live under an isolated tmp dir, so
/// restore lands in a place the test can inspect without touching shared paths.
fn tmp_config(agent_type: &str, tmp: &tempfile::TempDir) -> crate::config::Config {
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
async fn combined_archive() -> Vec<u8> {
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

#[tokio::test]
async fn download_on_start_restores_claude_and_codex_session_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    tokio::fs::create_dir_all(tmp.path().join("workspace"))
        .await
        .unwrap();
    let config = Arc::new(tmp_config("claude-code", &tmp));

    let backend = ReplayBackend::new(Ok(vec![]), None);
    backend.with_download_tar(combined_archive().await);
    let (state, _rx) = state_from_config(Arc::clone(&config), InitPhase::Starting, backend);

    super::transcript_sync::download_on_start(&state).await;

    let claude_dst = super::transcript_sync::transcript_dir(&config);
    let codex_dst = super::transcript_sync::codex_sessions_dir(&config);
    assert_eq!(
        tokio::fs::read_to_string(claude_dst.join("session.jsonl"))
            .await
            .unwrap(),
        "{\"turn\":1}\n"
    );
    assert_eq!(
        tokio::fs::read_to_string(codex_dst.join("2026").join("rollout-xyz.jsonl"))
            .await
            .unwrap(),
        "{\"thread\":\"xyz\"}\n"
    );
}

#[tokio::test]
async fn download_on_start_is_a_noop_when_no_archive_is_stored() {
    // A missing archive (fresh task, or S3 empty) restores nothing but still
    // creates the dirs, and must not fail — the CLI then legitimately has no
    // session to resume.
    let tmp = tempfile::TempDir::new().unwrap();
    tokio::fs::create_dir_all(tmp.path().join("workspace"))
        .await
        .unwrap();
    let config = Arc::new(tmp_config("codex", &tmp));

    let backend = ReplayBackend::new(Ok(vec![]), None); // download_tar left empty
    let (state, _rx) = state_from_config(Arc::clone(&config), InitPhase::Starting, backend);

    super::transcript_sync::download_on_start(&state).await;

    let codex_dst = super::transcript_sync::codex_sessions_dir(&config);
    let mut entries = tokio::fs::read_dir(&codex_dst).await.unwrap();
    assert!(
        entries.next_entry().await.unwrap().is_none(),
        "no archive → nothing restored"
    );
}

#[tokio::test]
async fn download_on_start_survives_a_corrupt_archive() {
    // A corrupt archive must be non-fatal and restore nothing (so a subsequent
    // `--resume` hits a genuinely missing session and the coordinator's
    // SessionImpact::Lost recovery takes over — see the coordinator tests).
    let tmp = tempfile::TempDir::new().unwrap();
    tokio::fs::create_dir_all(tmp.path().join("workspace"))
        .await
        .unwrap();
    let config = Arc::new(tmp_config("claude-code", &tmp));

    let backend = ReplayBackend::new(Ok(vec![]), None);
    backend.with_download_tar(b"not a tar archive at all".to_vec());
    let (state, _rx) = state_from_config(Arc::clone(&config), InitPhase::Starting, backend);

    // Must not panic.
    super::transcript_sync::download_on_start(&state).await;

    let claude_dst = super::transcript_sync::transcript_dir(&config);
    let mut entries = tokio::fs::read_dir(&claude_dst).await.unwrap();
    assert!(
        entries.next_entry().await.unwrap().is_none(),
        "a corrupt archive must restore nothing"
    );
}
