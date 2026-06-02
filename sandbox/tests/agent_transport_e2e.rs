//! E2E test of the production agent path (`spawn_agent`) over BOTH SDK transports.
//!
//! The live agent runtime is the actor spawned by
//! `moatless_sandbox::agent::handle::spawn_agent`. The transport switch lives in
//! its `start_session` → `sdk_query` closure, driven by `Config.use_tmux`:
//! `use_tmux = false` → `claude_agent_sdk::query()` (subprocess / NDJSON `-p`);
//! `use_tmux = true` → `claude_agent_sdk::query_tmux()` (tmux / TUI). Both
//! branches share the same `build_options()`.
//!
//! These tests drive the actor end-to-end with a no-op `BackendClient` and a
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
//! The agent binary is resolved from `CLAUDE_CLI_PATH` (default
//! `/usr/local/bin/claude`), and the auth token from `CLAUDE_CODE_OAUTH_TOKEN`
//! or `ANTHROPIC_API_KEY` — set so the tests can run hermetically inside the
//! sandbox image (which ships both the CLI and `tmux`).

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
use moatless_sandbox::agent::handle::{spawn_agent, AgentHandle};
use moatless_sandbox::agent::types::AgentStatus;
use moatless_sandbox::backend_client::BackendClient;
use moatless_sandbox::config::{AgentConfig, Config, ProxyConfig};
use moatless_sandbox::credentials::recovery::CredentialFailureDetector;
use moatless_sandbox::credentials::Dispatcher;
use moatless_sandbox::events::GrpcEventPublisher;

use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingMessage,
    PublishEventRequest,
};
use tempfile::TempDir;
use tokio::sync::RwLock;

/// Cheapest model — keeps E2E cost low.
const TEST_MODEL: &str = "claude-haiku-4-5-20251001";
/// A no-tool prompt. `build_options` installs an interactive permission handler
/// (NOT `--dangerously-skip-permissions`), and the tmux transport IGNORES the
/// permission handler entirely — so a tool-using prompt would stall on a
/// permission gate. A pure arithmetic reply needs no tools.
const TEST_PROMPT: &str = "What is 2+2? Reply with only the number.";

// ── Auth token from the host environment ────────────────────────────────────

/// Auth credentials forwarded to the agent subprocess. The actor's
/// `start_session` calls `BackendClient::get_credentials` and OVERWRITES the
/// in-pod credentials map with the result, so the token must be returned here
/// (preloading the `credentials` RwLock alone would be clobbered).
/// `filter_agent_credentials` retains both Claude auth keys.
fn auth_credentials() -> HashMap<String, String> {
    let mut creds = HashMap::new();
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        creds.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token);
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        creds.insert("ANTHROPIC_API_KEY".to_string(), key);
    }
    creds
}

// ── No-op backend ───────────────────────────────────────────────────────────

/// Minimal `BackendClient` for driving the actor without a real backend.
/// `get_credentials` returns the host auth token so it reaches the agent env.
struct NoopBackend {
    credentials: HashMap<String, String>,
}

#[async_trait::async_trait]
impl BackendClient for NoopBackend {
    async fn publish_event(&self, _req: PublishEventRequest) {}

    async fn get_pending_messages(
        &self,
        _task_id: &str,
    ) -> Result<Vec<PendingMessage>, tonic::Status> {
        Ok(vec![])
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _message_uuids: Vec<String>,
        _session_id: &str,
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
}

// ── Actor wiring ────────────────────────────────────────────────────────────

fn test_config(tmp: &TempDir, use_tmux: bool) -> Config {
    let home = tmp.path().join("home");
    Config {
        port: 0,
        grpc_port: 0,
        task_id: "agent-transport-e2e".to_string(),
        workspace_dir: tmp.path().join("workspace").display().to_string(),
        data_dir: tmp.path().join("data").display().to_string(),
        upload_dir: tmp.path().join("uploads").display().to_string(),
        log_level: "info".to_string(),
        backend_grpc_url: "http://localhost:50051".to_string(),
        backend_api_url: None,
        user_auth_token: None,
        agent_config: AgentConfig::default(),
        claude_cli_path: std::env::var("CLAUDE_CLI_PATH")
            .unwrap_or_else(|_| "/usr/local/bin/claude".to_string()),
        codex_cli_path: "/usr/local/bin/codex".to_string(),
        proxy: ProxyConfig::default(),
        servers: vec![],
        frontend_url: None,
        default_model: None,
        default_codex_model: None,
        use_tmux,
        home_dir: home.display().to_string(),
        codex_home_dir: home.join(".codex").display().to_string(),
        claude_config_dir: home.join(".claude").display().to_string(),
        git_clone_url: None,
        git_branch: None,
        git_provider: "github".to_string(),
        repo_config_raw: None,
        setup_commands: vec![],
        plugin_ids: vec![],
        plugin_repos: vec![],
    }
}

/// Spawn the production agent actor wired with a no-op backend. Returns the
/// handle, the shared agent state, and the `TempDir` (kept alive by the caller).
fn spawn_test_agent(use_tmux: bool) -> (AgentHandle, Arc<AgentState>, TempDir) {
    use moatless_sandbox::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};

    let tmp = TempDir::new().expect("create tempdir");
    // start_session persists credential files under these dirs.
    for sub in ["workspace", "data", "uploads", "home/.claude", "home/.codex"] {
        std::fs::create_dir_all(tmp.path().join(sub)).expect("create test dir");
    }

    let backend: Arc<dyn BackendClient> = Arc::new(NoopBackend {
        credentials: auth_credentials(),
    });

    let events = GrpcEventPublisher::new(Arc::clone(&backend));
    let config = Arc::new(test_config(&tmp, use_tmux));
    let agent_state = Arc::new(AgentState::new());
    let credentials = Arc::new(RwLock::new(HashMap::new()));

    let late_injector = Arc::new(LateInjector::new());
    let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::clone(&late_injector) as _));

    let credential_failures = CredentialFailureDetector::default();
    let credential_dispatcher = Arc::new(Dispatcher::new());
    let system_prompt_cache: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));

    let handle = spawn_agent(
        Arc::clone(&config),
        Arc::clone(&agent_state),
        events,
        Arc::clone(&credentials),
        Arc::clone(&backend),
        Arc::clone(&scheduled_jobs),
        credential_failures,
        Arc::clone(&credential_dispatcher),
        Arc::clone(&system_prompt_cache),
    );
    late_injector.set(Box::new(handle.clone()));

    (handle, agent_state, tmp)
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

async fn run_agent_roundtrip(use_tmux: bool) {
    init_tracing();
    let (handle, agent_state, _tmp) = spawn_test_agent(use_tmux);

    // `send` resolves on SessionInit (session started), not on completion.
    let ack = handle.send(
        TEST_PROMPT.to_string(),
        Some(TEST_MODEL.to_string()),
        None, // agent_type → claude-code
        None, // agent_mode
        None, // resume_session_id
    );
    let session_id = tokio::time::timeout(Duration::from_secs(90), ack)
        .await
        .expect("timed out waiting for session start")
        .expect("session failed to start (ack sender dropped)");
    assert!(!session_id.is_empty(), "expected a non-empty session id");

    // Completion is observed via state polling — `AgentHandle` has no
    // completion oneshot. The actor sets Idle (success) or Idle+last_error
    // (failure) after the SDK `Result` message.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if !matches!(agent_state.status().await, AgentStatus::Running) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "agent did not finish within timeout"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let error = agent_state.last_error().await;
    assert!(error.is_none(), "agent finished with error: {error:?}");
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    handle.stop();
}

// ── Tests ───────────────────────────────────────────────────────────────────

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

/// Tmux transport. Gated on `CLAUDE_TMUX_E2E=1` and requires `tmux` on PATH.
#[tokio::test]
#[ignore]
async fn test_spawn_agent_tmux() {
    if std::env::var("CLAUDE_TMUX_E2E").as_deref() != Ok("1") {
        eprintln!("Skipping tmux E2E test (set CLAUDE_TMUX_E2E=1 to run)");
        return;
    }
    if std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .is_err()
    {
        eprintln!("Skipping tmux E2E test (tmux binary not found on PATH)");
        return;
    }
    run_agent_roundtrip(true).await;
}
