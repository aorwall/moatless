//! E2E test of the production agent path (`spawn_agent`) over BOTH SDK transports.
//!
//! The live agent runtime is the coordinator service spawned by
//! `moatless_sandbox::agent::handle::spawn_agent`. The transport switch lives in
//! the Claude harness, driven by `Config.use_tmux`:
//! `use_tmux = false` → `claude_agent_sdk::query()` (subprocess / NDJSON `-p`,
//! one process per turn); `use_tmux = true` → `claude_agent_sdk::query_tmux_streaming()`
//! (tmux / TUI, persistent session — the TUI survives turn ends and follow-up
//! sends are pasted into the live session). Both branches share the same
//! `build_options()`.
//!
//! These tests drive the coordinator end-to-end with a no-op `BackendClient` and a
//! real `claude` CLI, asserting the agent reaches `Idle` with no error.
//!
//! Gating (mirrors `crates/claude-agent-sdk/tests/e2e.rs`): the subprocess test
//! runs under `CLAUDE_SDK_E2E=1`, the tmux test under `CLAUDE_TMUX_E2E=1` (and
//! requires `tmux` on PATH). Both are `#[ignore]`, so run with `-- --ignored`.
//!
//! Run:
//! `CLAUDE_SDK_E2E=1 CLAUDE_CODE_OAUTH_TOKEN=... cargo test -p moatless-sandbox
//! --test agent_transport_e2e -- --ignored --nocapture` (use `CLAUDE_TMUX_E2E=1`
//! for the tmux variant).
//!
//! The agent binaries are resolved from `CLAUDE_CLI_PATH` and `CODEX_CLI_PATH`
//! (defaults `/usr/local/bin/claude` and `/usr/local/bin/codex`), and the auth
//! token from `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` — set so the
//! tests can run hermetically inside the sandbox image (which ships both CLIs
//! and `tmux`). Point the two path vars at a candidate build to check a CLI
//! upgrade before its pin lands in `docker/cli-versions.env`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::ignore_without_reason
)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moatless_sandbox::agent::agent_state::AgentState;
use moatless_sandbox::agent::handle::{AgentHandle, spawn_agent};
use moatless_sandbox::agent::harness::{
    ClaudeConfig, ErrorClass, Harness, HarnessError, Input, Outcome, Rejection, SessionImpact,
    Signal,
};
use moatless_sandbox::agent::types::{AgentStatus, ResumeSession, StopSource};
use moatless_sandbox::backend_client::BackendClient;
use moatless_sandbox::config::{AgentConfig, Config, ProxyConfig};
use moatless_sandbox::credentials::Dispatcher;
use moatless_sandbox::credentials::recovery::CredentialFailureDetector;
use moatless_sandbox::events::GrpcEventPublisher;

use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery,
    PublishEventRequest,
};
use tempfile::TempDir;
use tokio::sync::RwLock;

/// Cheapest model — keeps E2E cost low. `CLAUDE_TEST_MODEL` overrides it to
/// check an upgraded CLI against the model a deployment actually runs.
const DEFAULT_CLAUDE_TEST_MODEL: &str = "claude-haiku-4-5-20251001";

fn claude_test_model() -> String {
    std::env::var("CLAUDE_TEST_MODEL")
        .unwrap_or_else(|_| DEFAULT_CLAUDE_TEST_MODEL.to_string())
}

/// A no-tool prompt. `build_options` installs an interactive permission handler
/// (NOT `--dangerously-skip-permissions`), and the tmux transport IGNORES the
/// permission handler entirely — so a tool-using prompt would stall on a
/// permission gate. A pure arithmetic reply needs no tools.
const TEST_PROMPT: &str = "What is 2+2? Reply with only the number.";

/// Codex model under test. `None` lets `codex_app_server::resolve_codex_model` fall
/// through to `DEFAULT_CODEX_MODEL`; override with `CODEX_TEST_MODEL` to pin a
/// cheaper model.
fn codex_test_model() -> Option<String> {
    std::env::var("CODEX_TEST_MODEL").ok()
}

// ── Auth token from the host environment ────────────────────────────────────

/// Auth credentials forwarded to the agent subprocess. The coordinator's input
/// build calls `BackendClient::get_credentials` and OVERWRITES the
/// in-pod credentials map with the result, so the token must be returned here
/// (preloading the `credentials` `RwLock` alone would be clobbered).
/// `filter_agent_credentials` retains both Claude auth keys and the Codex keys
/// (`OPENAI_API_KEY` / `CODEX_AUTH_JSON`); each executor uses only what it
/// needs, so returning the superset is harmless for both the Claude and Codex
/// paths.
fn auth_credentials() -> HashMap<String, String> {
    let mut creds = HashMap::new();
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        creds.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token);
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        creds.insert("ANTHROPIC_API_KEY".to_string(), key);
    }
    // Codex CLI auth: an API key is forwarded to the app-server child env;
    // CODEX_AUTH_JSON (ChatGPT-account tokens) is written to ~/.codex/auth.json.
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        creds.insert("OPENAI_API_KEY".to_string(), key);
    }
    if let Ok(json) = std::env::var("CODEX_AUTH_JSON") {
        creds.insert("CODEX_AUTH_JSON".to_string(), json);
    }
    creds
}

// ── No-op backend ───────────────────────────────────────────────────────────

/// Minimal `BackendClient` for driving the coordinator without a real backend.
/// `get_credentials` returns the host auth token so it reaches the agent env.
/// Published events are recorded as `"<event_type>/<sub_type>"` strings so
/// tests can assert on the emitted event sequence (poll-based status checks
/// can miss sub-millisecond Running windows when the tmux tailer delivers a
/// whole turn in one batch).
struct NoopBackend {
    credentials: HashMap<String, String>,
    events: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl BackendClient for NoopBackend {
    async fn publish_event(&self, req: PublishEventRequest) -> Result<(), tonic::Status> {
        self.events
            .lock()
            .expect("event log poisoned")
            .push(format!(
                "{}/{}",
                req.event_type().as_str_name(),
                req.sub_type
            ));
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
        Ok(GetCredentialsResponse {
            credentials: self.credentials.clone(),
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

// ── Actor wiring ────────────────────────────────────────────────────────────

fn test_config(tmp: &TempDir, use_tmux: bool, agent_type: &str) -> Config {
    let home = tmp.path().join("home");
    Config {
        port: 0,
        grpc_port: 0,
        task_id: "agent-transport-e2e".to_string(),
        sandbox_id: "agent-transport-e2e".to_string(),
        // The Sandbox's fixed harness. `None` reaches `resolve_agent_harness` as
        // `""`, which it rejects — every session then fails to start with
        // `invalid_configuration` before any CLI is spawned.
        agent_type: Some(agent_type.to_string()),
        workspace_dir: tmp.path().join("workspace").display().to_string(),
        repo_dir: tmp.path().join("workspace").display().to_string(),
        data_dir: tmp.path().join("data").display().to_string(),
        runtime_dir: tmp.path().join("run").display().to_string(),
        upload_dir: tmp.path().join("uploads").display().to_string(),
        log_level: "info".to_string(),
        backend_grpc_url: "http://localhost:50051".to_string(),
        backend_api_url: None,
        user_auth_token: None,
        bootstrap_token: None,
        agent_config: AgentConfig::default(),
        claude_cli_path: std::env::var("CLAUDE_CLI_PATH")
            .unwrap_or_else(|_| "/usr/local/bin/claude".to_string()),
        codex_cli_path: std::env::var("CODEX_CLI_PATH")
            .unwrap_or_else(|_| "/usr/local/bin/codex".to_string()),
        proxy: ProxyConfig::default(),
        servers: vec![],
        frontend_url: None,
        default_model: None,
        default_codex_model: None,
        openai_base_url: None,
        use_tmux,
        command_default_timeout_secs: moatless_sandbox::config::DEFAULT_COMMAND_TIMEOUT_SECS,
        home_dir: home.display().to_string(),
        codex_home_dir: home.join(".codex").display().to_string(),
        // Deliberately OUTSIDE `home/.claude` to mirror the cluster, where
        // CLAUDE_CONFIG_DIR=/opt/moatless/.claude but HOME=/home/moatless. This
        // is what makes the tmux JSONL-discovery path bug reproducible: the SDK
        // must resolve the transcript dir from CLAUDE_CONFIG_DIR, not $HOME.
        claude_config_dir: tmp.path().join("claude-config").display().to_string(),
        git_clone_url: None,
        git_branch: None,
        git_provider: "github".to_string(),
        repos: Vec::new(),
        repo_config_raw: None,
        setup_commands: vec![],
        plugins: vec![],
        permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
        workspace_sync_enabled: false,
        workspace_mirror_enabled: true,
        workspace_restore_if_empty: false,
        headless: false,
    }
}

/// Event log recorded by the no-op backend: `"<EVENT_TYPE>/<sub_type>"`.
type EventLog = Arc<std::sync::Mutex<Vec<String>>>;

/// Spawn the production agent lifecycle task wired with a no-op backend. Returns the
/// handle, the shared agent state, the published-event log, and the `TempDir`
/// (kept alive by the caller).
fn spawn_test_agent(
    use_tmux: bool,
    agent_type: &str,
) -> (AgentHandle, Arc<AgentState>, EventLog, TempDir) {
    let tmp = TempDir::new().expect("create tempdir");
    // The harness persists credential files under these dirs. `claude-config`
    // is the cold CLAUDE_CONFIG_DIR (outside HOME) — see `test_config`.
    for sub in [
        "workspace",
        "data",
        "uploads",
        "home/.claude",
        "home/.codex",
        "claude-config",
    ] {
        std::fs::create_dir_all(tmp.path().join(sub)).expect("create test dir");
    }
    // In a pod `init::credentials` turns CODEX_AUTH_JSON into this file; these
    // tests drive `spawn_agent` directly and never run init, so without it the
    // Codex CLI sees an empty CODEX_HOME and every turn 401s.
    if let Ok(auth_json) = std::env::var("CODEX_AUTH_JSON") {
        std::fs::write(tmp.path().join("home/.codex/auth.json"), auth_json)
            .expect("write Codex auth.json");
    }
    let (handle, agent_state, events) = spawn_test_agent_in(&tmp, use_tmux, agent_type);
    (handle, agent_state, events, tmp)
}

/// Spawn a fresh actor against an EXISTING test dir — used by the restart-
/// resume test to simulate a pod restart (new actor, same on-disk transcripts).
fn spawn_test_agent_in(
    tmp: &TempDir,
    use_tmux: bool,
    agent_type: &str,
) -> (AgentHandle, Arc<AgentState>, EventLog) {
    use moatless_sandbox::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};

    let event_log: EventLog = Arc::new(std::sync::Mutex::new(Vec::new()));

    let backend: Arc<dyn BackendClient> = Arc::new(NoopBackend {
        credentials: auth_credentials(),
        events: Arc::clone(&event_log),
    });

    let (outbox, _drainer) = moatless_sandbox::outbox::EventOutbox::spawn(
        Arc::clone(&backend),
        "test-task".to_string(),
        tokio_util::sync::CancellationToken::new(),
    );
    let events = GrpcEventPublisher::new(outbox);
    let config = Arc::new(test_config(tmp, use_tmux, agent_type));

    // Mirror the cluster: the `claude` child inherits CLAUDE_CONFIG_DIR from the
    // process env (the sandbox does NOT pass it through `options.env`), and the
    // SDK's tmux JSONL discovery must resolve the same dir. Setting it here so it
    // differs from $HOME/.claude is what reproduces the cold-config-dir bug — the
    // old SDK polled $HOME/.claude/projects and timed out with JsonlNotFound.
    std::env::set_var("CLAUDE_CONFIG_DIR", &config.claude_config_dir);
    let agent_state = Arc::new(AgentState::new());
    let credentials = Arc::new(RwLock::new(HashMap::new()));

    let late_injector = Arc::new(LateInjector::new());
    let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::clone(&late_injector) as _));

    let credential_failures = CredentialFailureDetector::default();
    let credential_dispatcher = Arc::new(Dispatcher::new());
    let system_prompt_cache: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));

    let commands = Arc::new(moatless_sandbox::commands::CommandRegistry::new(
        std::time::Duration::from_mins(1),
        std::env::temp_dir().join("moat-cmd-it"),
    ));

    let handle = spawn_agent(
        Arc::clone(&config),
        Arc::clone(&agent_state),
        events,
        Arc::clone(&credentials),
        Arc::clone(&backend),
        Arc::clone(&scheduled_jobs),
        commands,
        credential_failures,
        Arc::clone(&credential_dispatcher),
        Arc::clone(&system_prompt_cache),
        moatless_sandbox::agent::heartbeat::Heartbeat::new(),
        moatless_sandbox::init::workspace_sync::WorkspaceSyncTracker::new(),
    );
    late_injector.set(Box::new(handle.clone()));

    (handle, agent_state, event_log)
}

// ── Shared run body ─────────────────────────────────────────────────────────

/// Initialize tracing for test output (idempotent).
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("moatless_sandbox=debug".parse().unwrap())
                .add_directive("claude_agent_sdk=debug".parse().unwrap()),
        )
        .with_test_writer()
        .try_init();
}

/// Poll until the agent leaves `Running` (turn finished). Completion is
/// observed via state polling — `AgentHandle` has no completion oneshot. The
/// actor sets Idle (success) or `Idle+last_error` (failure) after the SDK
/// `Result` message.
async fn wait_until_not_running(agent_state: &AgentState, timeout: Duration, what: &str) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !matches!(agent_state.status().await, AgentStatus::Running) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: agent did not finish within timeout"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn run_agent_roundtrip(use_tmux: bool) {
    init_tracing();
    let (handle, agent_state, events, _tmp) = spawn_test_agent(use_tmux, "claude-code");

    // `send` resolves on SessionInit (session started), not on completion.
    let ack = handle.send(
        TEST_PROMPT.to_string(),
        Some(claude_test_model()),
        None,  // agent_type → claude-code
        None,  // agent_mode
        None,  // effort
        None,  // resume_session_id
        false, // start_fresh
        None,  // fork
    );
    let ack = tokio::time::timeout(Duration::from_secs(90), ack)
        .await
        .expect("timed out waiting for session start")
        .expect("session failed to start (ack sender dropped)")
        .session_id;
    assert!(!ack.is_empty(), "expected a non-empty session id");

    wait_until_not_running(&agent_state, Duration::from_mins(2), "roundtrip").await;

    let error = agent_state.last_error().await;
    assert!(error.is_none(), "agent finished with error: {error:?}");
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    // Regression guard: a first turn on a brand-new task must be a clean single
    // spawn — no `--continue` fallback failing and retrying fresh (which would
    // publish `session_context_lost` and wrap the prompt in context recovery).
    let published = events.lock().expect("event log poisoned").clone();
    assert!(
        !published.iter().any(|e| e.contains("session_context_lost")),
        "first turn must not report lost session context: {published:?}"
    );

    handle.stop(StopSource::User);
}

/// Codex roundtrip body: drives the production `spawn_agent` path with
/// `agent_type = "codex"`, which routes through `codex_app_server::start` — the
/// real `codex app-server --stdio` subprocess used in the cluster (NOT the
/// standalone in-process `codex-sdk` crate). Codex ignores `use_tmux` and always
/// runs one process per turn, so this mirrors `run_agent_roundtrip` without the
/// tmux/`session_context_lost` assertions (both are Claude-specific).
async fn run_codex_roundtrip() {
    init_tracing();
    let (handle, agent_state, _events, _tmp) = spawn_test_agent(false, "codex");

    let ack = handle.send(
        TEST_PROMPT.to_string(),
        codex_test_model(),
        Some("codex".to_string()),
        None,  // agent_mode
        None,  // effort
        None,  // resume_session_id
        false, // start_fresh
        None,  // fork
    );
    let session_id = tokio::time::timeout(Duration::from_secs(90), ack)
        .await
        .expect("timed out waiting for Codex session start")
        .expect("Codex session failed to start (ack sender dropped)")
        .session_id;
    assert!(
        !session_id.is_empty(),
        "expected a non-empty Codex session id"
    );

    // Codex's first turn (cold auth + model warmup) can be slower than Claude.
    wait_until_not_running(&agent_state, Duration::from_mins(3), "codex roundtrip").await;

    let error = agent_state.last_error().await;
    assert!(
        error.is_none(),
        "Codex agent finished with error: {error:?}"
    );
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    handle.stop(StopSource::User);
}

/// Restart-resume body: turn 1 runs on actor A, then A is stopped (killing the
/// CLI session — the pod-restart analogue) and turn 2 runs on a FRESH actor B
/// with `resume_session_id`. This exercises the tmux JSONL-discovery resume
/// path — on `claude --resume <id>` claude appends to the existing
/// `<id>.jsonl` (no new file), so discovery must target the known transcript
/// directly instead of waiting for a new file (which times out with
/// `JsonlNotFound`).
async fn run_agent_resume(use_tmux: bool) {
    init_tracing();
    let (handle, agent_state, _events, tmp) = spawn_test_agent(use_tmux, "claude-code");

    // ── Turn 1: fresh session on actor A ────────────────────────────────────
    let ack = handle.send(
        TEST_PROMPT.to_string(),
        Some(claude_test_model()),
        None,  // agent_type → claude-code
        None,  // agent_mode
        None,  // effort
        None,  // resume_session_id → fresh
        false, // start_fresh
        None,  // fork
    );
    let session_id = tokio::time::timeout(Duration::from_secs(90), ack)
        .await
        .expect("turn 1: timed out waiting for session start")
        .expect("turn 1: session failed to start (ack sender dropped)")
        .session_id;
    assert!(!session_id.is_empty(), "expected a non-empty session id");

    wait_until_not_running(&agent_state, Duration::from_mins(2), "turn 1").await;
    let error = agent_state.last_error().await;
    assert!(error.is_none(), "turn 1 finished with error: {error:?}");

    // ── Simulated pod restart: stop actor A, spawn actor B on the same dirs ──
    handle.stop(StopSource::User);
    let (handle, agent_state, _events2) = spawn_test_agent_in(&tmp, use_tmux, "claude-code");

    // ── Turn 2: resume the turn-1 session by id on actor B ───────────────────
    let ack = handle.send(
        "What is 3+3? Reply with only the number.".to_string(),
        Some(claude_test_model()),
        None,                     // agent_type → claude-code
        None,                     // agent_mode
        None,                     // effort
        Some(session_id.clone()), // resume_session_id → resume turn 1
        false,                    // start_fresh
        None,                     // fork
    );
    let resumed_id = tokio::time::timeout(Duration::from_secs(90), ack)
        .await
        .expect("turn 2: timed out waiting for session start")
        .expect("turn 2: session failed to start (ack sender dropped)")
        .session_id;
    assert!(
        !resumed_id.is_empty(),
        "expected a non-empty resumed session id"
    );

    wait_until_not_running(&agent_state, Duration::from_mins(2), "turn 2").await;

    let error = agent_state.last_error().await;
    assert!(
        error.is_none(),
        "turn 2 (resume) finished with error: {error:?}"
    );
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    handle.stop(StopSource::User);
}

/// Multi-turn persistent-session body (tmux): one lifecycle task, two sends. Turn 1
/// spawns the persistent TUI; turn 2 must be pasted into the SAME live session
/// (same `session_id`, no respawn) — the coordinator keeps the handle alive across
/// the turn boundary.
async fn run_agent_multi_turn(use_tmux: bool) {
    init_tracing();
    let (handle, agent_state, _events, _tmp) = spawn_test_agent(use_tmux, "claude-code");

    // ── Turn 1 ───────────────────────────────────────────────────────────────
    let ack = handle.send(
        TEST_PROMPT.to_string(),
        Some(claude_test_model()),
        None,
        None,
        None,
        None,
        false,
        None, // fork
    );
    let session_id = tokio::time::timeout(Duration::from_secs(90), ack)
        .await
        .expect("turn 1: timed out waiting for session start")
        .expect("turn 1: session failed to start (ack sender dropped)")
        .session_id;
    assert!(!session_id.is_empty(), "expected a non-empty session id");

    wait_until_not_running(&agent_state, Duration::from_mins(2), "turn 1").await;
    let error = agent_state.last_error().await;
    assert!(error.is_none(), "turn 1 finished with error: {error:?}");

    // ── Turn 2: pasted into the live session ─────────────────────────────────
    let ack = handle.send(
        "What is 3+3? Reply with only the number.".to_string(),
        Some(claude_test_model()),
        None,
        None,
        None,
        None,
        false,
        None, // fork
    );
    let second_id = tokio::time::timeout(Duration::from_secs(90), ack)
        .await
        .expect("turn 2: timed out waiting for ack")
        .expect("turn 2: ack sender dropped")
        .session_id;
    assert_eq!(
        second_id, session_id,
        "turn 2 must run in the same persistent session"
    );

    wait_until_not_running(&agent_state, Duration::from_mins(2), "turn 2").await;
    let error = agent_state.last_error().await;
    assert!(error.is_none(), "turn 2 finished with error: {error:?}");
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    handle.stop(StopSource::User);
}

/// CLI-internal wakeup body (tmux): the agent is told to call `ScheduleWakeup`
/// with a short delay. In tmux mode the sandbox does NOT arm its own wakeup
/// job — the CLI's internal timer fires inside the persistent TUI and starts
/// a turn on its own. The actor must detect that turn and publish a full
/// turn cycle (`agent/started` + `taskTurn/active` + `agent/completed`).
///
/// Completion is asserted on the published-event log rather than status
/// polling: the tmux tailer can deliver the whole internal turn in one batch,
/// making the `Running` window sub-millisecond and unpollable.
async fn run_agent_internal_wakeup(use_tmux: bool) {
    init_tracing();
    let (handle, agent_state, events, _tmp) = spawn_test_agent(use_tmux, "claude-code");

    let count = |needle: &str| {
        events
            .lock()
            .expect("event log poisoned")
            .iter()
            .filter(|e| e.as_str() == needle)
            .count()
    };

    let ack = handle.send(
        "Call the ScheduleWakeup tool exactly once with delaySeconds=5, \
         prompt=\"Reply with only the word DONE\", reason=\"e2e test\". \
         After calling it, reply with only the word SCHEDULED."
            .to_string(),
        Some(claude_test_model()),
        None,
        None,
        None,
        None,
        false,
        None, // fork
    );
    let session_id = tokio::time::timeout(Duration::from_secs(90), ack)
        .await
        .expect("turn 1: timed out waiting for session start")
        .expect("turn 1: session failed to start (ack sender dropped)")
        .session_id;
    assert!(!session_id.is_empty(), "expected a non-empty session id");

    wait_until_not_running(&agent_state, Duration::from_mins(2), "turn 1").await;
    let error = agent_state.last_error().await;
    assert!(error.is_none(), "turn 1 finished with error: {error:?}");
    assert_eq!(count("EVENT_TYPE_AGENT/completed"), 1, "turn 1 completed");

    // The CLI clamps `delaySeconds` to a 60s minimum (next minute boundary,
    // so up to ~2min), then fires the wakeup turn inside the live TUI. Wait
    // for the coordinator to surface it as a full second turn cycle.
    let deadline = tokio::time::Instant::now() + Duration::from_mins(4);
    loop {
        if count("EVENT_TYPE_AGENT/completed") >= 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "internal wakeup turn never completed; events: {:?}",
            events.lock().expect("event log poisoned")
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // The internally-started turn must have published its own turn-start pair.
    assert!(
        count("EVENT_TYPE_AGENT/started") >= 2,
        "expected a second agent/started for the CLI-internal turn; events: {:?}",
        events.lock().expect("event log poisoned")
    );
    assert!(
        count("EVENT_TYPE_TASK_TURN/active") >= 2,
        "expected a second taskTurn/active for the CLI-internal turn; events: {:?}",
        events.lock().expect("event log poisoned")
    );

    let error = agent_state.last_error().await;
    assert!(
        error.is_none(),
        "internal wakeup turn finished with error: {error:?}"
    );
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    handle.stop(StopSource::User);
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// True when the tmux E2E tier is enabled (`CLAUDE_TMUX_E2E=1`) AND a `tmux`
/// binary is available on PATH. Prints the skip reason otherwise.
fn tmux_e2e_enabled(what: &str) -> bool {
    if std::env::var("CLAUDE_TMUX_E2E").as_deref() != Ok("1") {
        eprintln!("Skipping {what} (set CLAUDE_TMUX_E2E=1 to run)");
        return false;
    }
    if std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .is_err()
    {
        eprintln!("Skipping {what} (tmux binary not found on PATH)");
        return false;
    }
    true
}

/// Subprocess transport (`-p` / NDJSON). Gated on `CLAUDE_SDK_E2E=1`.
#[tokio::test]
#[ignore]
async fn test_spawn_agent_subprocess() {
    if std::env::var("CLAUDE_SDK_E2E").as_deref() != Ok("1") {
        eprintln!("Skipping subprocess E2E test (set CLAUDE_SDK_E2E=1 to run)");
        return;
    }
    run_agent_roundtrip(false).await;
}

/// The causal CLI-spawn join for the missing/corrupt-restore → `Lost` chain
/// (Sandbox-recovery testing decision, restore.rs link 1→2): a restore that
/// leaves no session on disk, then a real `--resume` against that now-absent
/// transcript, must have the *production* Claude harness classify the CLI's
/// startup failure as `(UnavailableSession, Lost)` — not merely start a fresh
/// session. The classification is unit-tested in `claude::classify`; this drives
/// the real binary so the spawn→classify join is exercised end to end.
async fn run_missing_session_resume_is_lost() {
    let tmp = TempDir::new().expect("create tempdir");
    for sub in [
        "workspace",
        "data",
        "uploads",
        "home/.claude",
        "claude-config",
    ] {
        tokio::fs::create_dir_all(tmp.path().join(sub))
            .await
            .expect("create dir");
    }
    let config = test_config(&tmp, false, "claude-code");

    // The absent restore target: the fresh, isolated CLAUDE_CONFIG_DIR holds no
    // sessions, so `projects/` (where the CLI writes and looks up transcripts,
    // per `transcript_sync::transcript_dir`) does not exist and the resumed
    // session id cannot resolve to any transcript.
    let projects_dir = std::path::Path::new(&config.claude_config_dir).join("projects");
    assert!(
        !tokio::fs::try_exists(&projects_dir)
            .await
            .expect("inspect isolated Claude project directory"),
        "the isolated session store must contain no transcript to resume"
    );

    let mut harness = Harness::claude(ClaudeConfig {
        config: Arc::new(config),
        agent_state: Arc::new(AgentState::new()),
        agent_type: Some("claude-code".to_string()),
        events: None,
    });

    // A syntactically valid session id with no transcript under the isolated
    // store above — so the resume target is provably absent.
    harness
        .submit(Input {
            resume: Some(ResumeSession::WithId(
                "00000000-0000-0000-0000-000000000000".to_string(),
            )),
            ..Input::text("resume-missing", "Continue the prior conversation.")
        })
        .await
        .expect("submit to open harness");

    // The startup failure surfaces as a classified error, whether the harness
    // rejects the input pre-start or the opened execution fails.
    let err = tokio::time::timeout(Duration::from_secs(90), async {
        while let Some(signal) = harness.next_signal().await {
            match signal {
                Signal::InputRejected {
                    reason: Rejection::Failed(err),
                    ..
                }
                | Signal::ExecutionFinished {
                    outcome: Outcome::Failed(err),
                } => return err,
                other => eprintln!("resume-missing: intermediate signal {other:?}"),
            }
        }
        panic!("harness closed without a classified failure for the absent resume");
    })
    .await
    .expect("resume-missing classification timed out");

    let HarnessError {
        class,
        session_impact,
        diagnostic,
        ..
    } = err;
    assert_eq!(
        session_impact,
        SessionImpact::Lost,
        "resuming an absent transcript must classify the Session as Lost (got {diagnostic})"
    );
    assert_eq!(
        class,
        ErrorClass::UnavailableSession,
        "an absent resume target is an unavailable Session (got {diagnostic})"
    );
}

/// Missing-restore → `Lost` through the production harness. The harness rejects
/// the absent restored transcript before spawning the CLI, so this regression is
/// deterministic and needs neither credentials nor an installed executable.
#[tokio::test]
async fn test_missing_session_resume_classifies_lost() {
    run_missing_session_resume_is_lost().await;
}

/// Codex CLI transport (`codex app-server --stdio` subprocess). Gated on `CODEX_E2E=1`.
/// Needs the `codex` CLI on PATH (baked into the sandbox base image) and
/// `OPENAI_API_KEY` (or `CODEX_AUTH_JSON`).
#[tokio::test]
#[ignore]
async fn test_spawn_agent_codex() {
    if std::env::var("CODEX_E2E").as_deref() != Ok("1") {
        eprintln!("Skipping Codex E2E test (set CODEX_E2E=1 to run)");
        return;
    }
    run_codex_roundtrip().await;
}

/// Tmux transport. Gated on `CLAUDE_TMUX_E2E=1` and requires `tmux` on PATH.
#[tokio::test]
#[ignore]
async fn test_spawn_agent_tmux() {
    if !tmux_e2e_enabled("tmux E2E test") {
        return;
    }
    run_agent_roundtrip(true).await;
}

/// Tmux restart-resume. Turn 1 on actor A; A is stopped (pod-restart
/// analogue); turn 2 on a fresh actor B resumes by `session_id`, guarding
/// against the `JsonlNotFound` resume-discovery regression.
#[tokio::test]
#[ignore]
async fn test_resume_agent_tmux() {
    if !tmux_e2e_enabled("tmux resume E2E test") {
        return;
    }
    run_agent_resume(true).await;
}

/// Tmux persistent multi-turn: two sends on one actor must share the same
/// live session (turn 2 is pasted into the running TUI, not respawned).
#[tokio::test]
#[ignore]
async fn test_multi_turn_agent_tmux() {
    if !tmux_e2e_enabled("tmux multi-turn E2E test") {
        return;
    }
    run_agent_multi_turn(true).await;
}

/// Tmux CLI-internal wakeup: the CLI's own `ScheduleWakeup` timer fires inside
/// the persistent TUI and starts a turn with no sandbox send; the coordinator must
/// surface and complete it.
#[tokio::test]
#[ignore]
async fn test_internal_wakeup_agent_tmux() {
    if !tmux_e2e_enabled("tmux internal-wakeup E2E test") {
        return;
    }
    run_agent_internal_wakeup(true).await;
}
