//! E2E test of the production agent path (`spawn_agent`) over both transports.
//!
//! `Config.use_tmux` switches between `claude_agent_sdk::query()` (subprocess,
//! one process per turn) and `query_tmux_streaming()` (persistent tmux/TUI
//! session); both share `build_options()`. Tests drive the coordinator with a
//! no-op `BackendClient` and a real `claude` CLI, asserting `Idle` with no
//! error. Gated behind `CLAUDE_SDK_E2E=1` / `CLAUDE_TMUX_E2E=1` and
//! `#[ignore]`; binaries resolve from `CLAUDE_CLI_PATH` / `CODEX_CLI_PATH`.

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
    ClaudeConfig, ErrorClass, HarnessError, HarnessRuntime, Input, Outcome, Rejection,
    SessionImpact, Signal,
};
use moatless_sandbox::agent::types::{AgentStatus, ResumeSession, StopSource};
use moatless_sandbox::backend_client::BackendClient;
use moatless_sandbox::config::{AgentConfig, Config};
use moatless_sandbox::credentials::Dispatcher;
use moatless_sandbox::credentials::recovery::CredentialFailureDetector;
use moatless_sandbox::events::GrpcEventPublisher;

use sandbox_proto::{GetCredentialsResponse, PublishEventRequest};
use tempfile::TempDir;
use tokio::sync::RwLock;

/// Cheapest model — keeps E2E cost low. `CLAUDE_TEST_MODEL` overrides it to
/// check an upgraded CLI against the model a deployment actually runs.
const DEFAULT_CLAUDE_TEST_MODEL: &str = "claude-haiku-4-5-20251001";

fn claude_test_model() -> String {
    std::env::var("CLAUDE_TEST_MODEL").unwrap_or_else(|_| DEFAULT_CLAUDE_TEST_MODEL.to_string())
}

/// A no-tool prompt.
///
/// `build_options` installs an interactive permission handler (not
/// `--dangerously-skip-permissions`), and the tmux transport ignores it
/// entirely, so a tool-using prompt would stall on a permission gate.
const TEST_PROMPT: &str = "What is 2+2? Reply with only the number.";

/// Codex model under test: the backend's default unless `CODEX_TEST_MODEL`
/// pins a cheaper one. The sandbox refuses a Codex turn without a model.
fn codex_test_model() -> String {
    std::env::var("CODEX_TEST_MODEL").unwrap_or_else(|_| {
        moatless_types::model::CodexModel::Gpt6Astra
            .id()
            .to_string()
    })
}

/// The proxy Codex routes through, read the way `Config::from_env` reads it.
///
/// `Some` puts the CLI on `OPENAI_API_KEY` against the proxy, like a pod;
/// `None` falls back to `api.openai.com` on `auth.json`'s ChatGPT tokens — a
/// path no deployment runs.
fn codex_base_url() -> Option<String> {
    std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
}

// ── Auth token from the host environment ────────────────────────────────────

/// Auth credentials forwarded to the agent subprocess.
///
/// The coordinator's input build calls `get_credentials` and overwrites the in-
/// pod credentials map, so preloading the `credentials` `RwLock` alone would be
/// clobbered; the token must come from here.
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
///
/// `get_credentials` returns the host auth token; events are recorded as
/// `"<event_type>/<sub_type>"` strings, since polling can miss sub-millisecond
/// Running windows when the tmux tailer delivers a whole turn in one batch.
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

    async fn get_credentials(
        &self,
        _task_id: &str,
    ) -> Result<GetCredentialsResponse, tonic::Status> {
        Ok(GetCredentialsResponse {
            credentials: self.credentials.clone(),
            ssh_keys: HashMap::new(),
        })
    }
}

// ── Actor wiring ────────────────────────────────────────────────────────────

fn test_config(tmp: &TempDir, use_tmux: bool, agent_profile: &str) -> Config {
    let home = tmp.path().join("home");
    Config {
        port: 0,
        grpc_port: 0,
        task_id: "agent-transport-e2e".to_string(),
        sandbox_id: "agent-transport-e2e".to_string(),
        // The Sandbox's fixed harness. `None` reaches `resolve_agent_harness` as
        // `""`, which it rejects — every session then fails to start with
        // `invalid_configuration` before any CLI is spawned.
        agent_profile: Some(agent_profile.to_string()),
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
        frontend_url: None,
        default_model: None,
        openai_base_url: codex_base_url(),
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
        github_upstreams: Default::default(),
        repo_config_raw: None,
        setup_commands: vec![],
        plugins: vec![],
        removed_plugins: Vec::new(),
        language_servers: vec![],
        search_path: None,
        http: moatless_sandbox::config::SandboxHttpConfig::default(),
        clone_stall_secs: moatless_sandbox::config::DEFAULT_CLONE_STALL_SECS,
        permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
        workspace_sync_enabled: false,
        workspace_restore_if_empty: false,
        headless: false,
        claude_disallowed_tools: None,
        credential_refresh_interval: std::time::Duration::from_mins(15),
        transcript_sync_interval: None,
        workspace_sync_interval: None,
        workspace_limit: None,
        shell: "/bin/bash".to_string(),
    }
}

/// Event log recorded by the no-op backend: `"<EVENT_TYPE>/<sub_type>"`.
type EventLog = Arc<std::sync::Mutex<Vec<String>>>;

/// Spawn the production agent lifecycle task wired with a no-op backend.
/// Returns the handle, the shared agent state, the published-event log, and the
/// `TempDir` (kept alive by the caller).
fn spawn_test_agent(
    use_tmux: bool,
    agent_profile: &str,
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
    // In a pod `init` writes both of these; these tests drive `spawn_agent`
    // directly and never run init. Codex reads its base URL from `config.toml`
    // alone — it has no `ANTHROPIC_BASE_URL` equivalent — so without the file
    // the CLI ignores `OPENAI_BASE_URL` and sends every turn to api.openai.com.
    std::fs::write(
        tmp.path().join("home/.codex/config.toml"),
        moatless_sandbox::init::codex_config::render_config_toml(codex_base_url().as_deref()),
    )
    .expect("write Codex config.toml");
    if let Ok(auth_json) = std::env::var("CODEX_AUTH_JSON") {
        std::fs::write(tmp.path().join("home/.codex/auth.json"), auth_json)
            .expect("write Codex auth.json");
    }
    let (handle, agent_state, events) = spawn_test_agent_in(&tmp, use_tmux, agent_profile);
    (handle, agent_state, events, tmp)
}

/// Spawn a fresh actor against an EXISTING test dir — used by the restart-
/// resume test to simulate a pod restart (new actor, same on-disk transcripts).
fn spawn_test_agent_in(
    tmp: &TempDir,
    use_tmux: bool,
    agent_profile: &str,
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
    let config = Arc::new(test_config(tmp, use_tmux, agent_profile));

    // Mirrors the cluster: the `claude` child inherits CLAUDE_CONFIG_DIR from
    // the process env, not `options.env`, and the SDK's tmux JSONL discovery
    // must resolve the same dir. Set it here, different from $HOME/.claude,
    // deliberately: that's what catches the SDK polling the wrong directory
    // and timing out with JsonlNotFound.
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
        "task-1".to_string(),
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
        moatless_sandbox::workspace_sync::WorkspaceSyncTracker::new(),
        moatless_sandbox::init::SetupGate::opened(),
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

/// Poll until the agent leaves `Running` (turn finished): `AgentHandle` has no
/// completion oneshot. The actor sets Idle (success) or `Idle+last_error`
/// (failure) after the SDK `Result` message.
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
        None,  // agent_profile → claude-code
        None,  // agent_mode
        None,  // effort
        None,  // resume_session_id
        false, // start_fresh
        None,  // fork
        false, // switch_harness
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

    drop(handle.stop(StopSource::User).await);
}

/// Drives `spawn_agent` with `agent_profile = "codex"`, routing through the real
/// `codex app-server --stdio` subprocess, not the standalone `codex-sdk` crate.
///
/// Codex ignores `use_tmux`; mirrors `run_agent_roundtrip` minus the
/// Claude-specific tmux/`session_context_lost` assertions.
async fn run_codex_roundtrip() {
    init_tracing();
    let (handle, agent_state, _events, _tmp) = spawn_test_agent(false, "codex");

    let ack = handle.send(
        TEST_PROMPT.to_string(),
        Some(codex_test_model()),
        Some("codex".to_string()),
        None,  // agent_mode
        None,  // effort
        None,  // resume_session_id
        false, // start_fresh
        None,  // fork
        false, // switch_harness
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

    drop(handle.stop(StopSource::User).await);
}

/// Restart-resume body: turn 1 runs on actor A, A is stopped (pod-restart
/// analogue), and turn 2 resumes via `resume_session_id` on a fresh actor B.
///
/// `claude --resume <id>` appends to the existing `<id>.jsonl`, so discovery
/// must target it directly or it times out with `JsonlNotFound`.
async fn run_agent_resume(use_tmux: bool) {
    init_tracing();
    let (handle, agent_state, _events, tmp) = spawn_test_agent(use_tmux, "claude-code");

    // ── Turn 1: fresh session on actor A ────────────────────────────────────
    let ack = handle.send(
        TEST_PROMPT.to_string(),
        Some(claude_test_model()),
        None,  // agent_profile → claude-code
        None,  // agent_mode
        None,  // effort
        None,  // resume_session_id → fresh
        false, // start_fresh
        None,  // fork
        false, // switch_harness
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
    drop(handle.stop(StopSource::User).await);
    let (handle, agent_state, _events2) = spawn_test_agent_in(&tmp, use_tmux, "claude-code");

    // ── Turn 2: resume the turn-1 session by id on actor B ───────────────────
    let ack = handle.send(
        "What is 3+3? Reply with only the number.".to_string(),
        Some(claude_test_model()),
        None,                     // agent_profile → claude-code
        None,                     // agent_mode
        None,                     // effort
        Some(session_id.clone()), // resume_session_id → resume turn 1
        false,                    // start_fresh
        None,                     // fork
        false,                    // switch_harness
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

    drop(handle.stop(StopSource::User).await);
}

/// Multi-turn persistent-session body (tmux): one lifecycle task, two sends.
///
/// Turn 1 spawns the persistent TUI; turn 2 must be pasted into the same live
/// session (same `session_id`, no respawn) — the coordinator keeps the handle
/// alive across the turn boundary.
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
        None,  // fork
        false, // switch_harness
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
        None,  // fork
        false, // switch_harness
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

    drop(handle.stop(StopSource::User).await);
}

/// CLI-internal wakeup body (tmux): the actor must detect and publish a turn
/// the CLI starts on its own, with no sandbox-initiated send.
///
/// Completion is asserted on the published-event log, not status polling (see
/// [`NoopBackend`]).
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
        None,  // fork
        false, // switch_harness
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

    drop(handle.stop(StopSource::User).await);
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

/// A restore with no session on disk, then a real `--resume`, must classify as
/// `(UnavailableSession, Lost)`, not start a fresh session.
///
/// The classification itself is unit-tested in `claude::classify`; this drives
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

    let mut harness = HarnessRuntime::claude(ClaudeConfig {
        config: Arc::new(config),
        agent_state: Arc::new(AgentState::new()),
        agent_profile: Some("claude-code".to_string()),
        events: None,
        trajectory: None,
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
/// the absent restored transcript before spawning the CLI, so this regression
/// is deterministic and needs neither credentials nor an installed executable.
#[tokio::test]
async fn test_missing_session_resume_classifies_lost() {
    run_missing_session_resume_is_lost().await;
}

/// Codex CLI transport (`codex app-server --stdio` subprocess). Gated on
/// `CODEX_E2E=1`. Needs the `codex` CLI on PATH (baked into the sandbox base
/// image) and `OPENAI_API_KEY` (or `CODEX_AUTH_JSON`).
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

/// Tmux restart-resume. Turn 1 on actor A; A is stopped (pod-restart analogue);
/// turn 2 on a fresh actor B resumes by `session_id`, guarding against the
/// `JsonlNotFound` resume-discovery regression.
#[tokio::test]
#[ignore]
async fn test_resume_agent_tmux() {
    if !tmux_e2e_enabled("tmux resume E2E test") {
        return;
    }
    run_agent_resume(true).await;
}

/// Tmux persistent multi-turn: two sends on one actor must share the same live
/// session (turn 2 is pasted into the running TUI, not respawned).
#[tokio::test]
#[ignore]
async fn test_multi_turn_agent_tmux() {
    if !tmux_e2e_enabled("tmux multi-turn E2E test") {
        return;
    }
    run_agent_multi_turn(true).await;
}

/// Tmux CLI-internal wakeup: the CLI's own `ScheduleWakeup` timer fires inside
/// the persistent TUI and starts a turn with no sandbox send; the coordinator
/// must surface and complete it.
#[tokio::test]
#[ignore]
async fn test_internal_wakeup_agent_tmux() {
    if !tmux_e2e_enabled("tmux internal-wakeup E2E test") {
        return;
    }
    run_agent_internal_wakeup(true).await;
}
