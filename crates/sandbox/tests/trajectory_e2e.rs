//! Real-session trajectory tests: record, rebuild, resume and fork real Claude
//! Code and Codex sessions through the production harness, against
//! `MemoryTrajectoryBackend`, and assert on what the model answers.
//!
//! `TRAJECTORY_E2E=1 cargo test -p moatless-sandbox --test trajectory_e2e -- --ignored --nocapture`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::too_many_lines
)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};
use std::time::Duration;

use moatless_sandbox::agent::agent_state::AgentState;
use moatless_sandbox::agent::harness::content::CanonicalContentBlock;
use moatless_sandbox::agent::harness::trajectory::{
    ClaudeSessionFormat, CodexSessionFormat, EventContent, RenderContext, SessionFormat,
};
use moatless_sandbox::agent::harness::{
    ClaudeConfig, CodexConfig, HarnessRuntime, Input, Outcome, Role, Signal,
};
use moatless_sandbox::agent::types::{ForkSource, ResumeSession};
use moatless_sandbox::backend_client::{BackendClient, TrajectoryRead};
use moatless_sandbox::config::{AgentConfig, Config};
use moatless_sandbox::credentials::recovery::CredentialFailureDetector;
use moatless_sandbox::trajectory_tail::{RecordTarget, SessionRecorder, TrajectoryRecording};
use serde_json::Value;
use tempfile::TempDir;

#[path = "support/memory_trajectory.rs"]
#[allow(dead_code, reason = "shared with the unit tests, which use the rest")]
mod memory;
use memory::{MemoryFork, MemoryTrajectoryBackend};

const TURN_TIMEOUT: Duration = Duration::from_mins(5);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Claude,
    Codex,
}

/// One isolated harness home: its own `CLAUDE_CONFIG_DIR` or `CODEX_HOME` and
/// workspace, shared by every Task the test runs.
struct RealEnv {
    kind: Kind,
    tmp: Arc<TempDir>,
    config: Arc<Config>,
}

fn enabled() -> bool {
    if std::env::var("TRAJECTORY_E2E").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipping: set TRAJECTORY_E2E=1 to run");
    false
}

/// The CLI must not think it runs inside another Claude Code session.
fn scrub_parent_session_env() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for key in [
            "CLAUDECODE",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CODE_EXECPATH",
            "CLAUDE_PID",
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
        ] {
            std::env::remove_var(key);
        }
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "moatless_sandbox=info".into()),
            )
            .with_test_writer()
            .try_init();
    });
}

impl RealEnv {
    fn claude() -> Option<Self> {
        enabled().then(|| Self::new(Kind::Claude))
    }

    fn codex() -> Option<Self> {
        enabled().then(|| Self::new(Kind::Codex))
    }

    fn new(kind: Kind) -> Self {
        scrub_parent_session_env();
        let mut tmp = TempDir::new().unwrap();
        tmp.disable_cleanup(std::env::var_os("TRAJECTORY_E2E_KEEP").is_some());
        for sub in [
            "workspace",
            "data",
            "run",
            "uploads",
            "home",
            "claude-config",
        ] {
            std::fs::create_dir_all(tmp.path().join(sub)).unwrap();
        }
        let home = tmp.path().join("home");
        let codex_home = home.join(".codex");
        std::fs::create_dir_all(&codex_home).unwrap();
        let base_url = std::env::var("OPENAI_BASE_URL")
            .ok()
            .filter(|u| !u.trim().is_empty());
        std::fs::write(
            codex_home.join("config.toml"),
            moatless_sandbox::init::codex_config::render_config_toml(base_url.as_deref()),
        )
        .unwrap();
        if let Ok(auth) = std::env::var("CODEX_AUTH_JSON") {
            std::fs::write(codex_home.join("auth.json"), auth).unwrap();
        }
        let workspace = tmp.path().join("workspace").display().to_string();
        let config = Config {
            port: 0,
            grpc_port: 0,
            task_id: "trajectory-e2e".to_string(),
            sandbox_id: "trajectory-e2e".to_string(),
            agent_profile: Some(
                match kind {
                    Kind::Claude => "claude-code",
                    Kind::Codex => "codex",
                }
                .to_string(),
            ),
            workspace_dir: workspace.clone(),
            repo_dir: workspace,
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
            openai_base_url: base_url,
            use_tmux: false,
            command_default_timeout_secs: moatless_sandbox::config::DEFAULT_COMMAND_TIMEOUT_SECS,
            home_dir: home.display().to_string(),
            codex_home_dir: codex_home.display().to_string(),
            claude_config_dir: tmp.path().join("claude-config").display().to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repos: Vec::new(),
            github_upstreams: Default::default(),
            repo_config_raw: None,
            setup_commands: vec![],
            plugins: vec![],
            http: moatless_sandbox::config::SandboxHttpConfig::default(),
            permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
            workspace_sync_enabled: false,
            workspace_restore_if_empty: false,
            headless: false,
            credential_refresh_interval: std::time::Duration::from_mins(15),
            transcript_sync_interval: None,
            workspace_sync_interval: None,
            workspace_limit: None,
            shell: "/bin/bash".to_string(),
        };
        Self {
            kind,
            tmp: Arc::new(tmp),
            config: Arc::new(config),
        }
    }

    /// The same workspace and homes, running the other harness.
    fn other(&self) -> Self {
        let (kind, profile) = match self.kind {
            Kind::Claude => (Kind::Codex, "codex"),
            Kind::Codex => (Kind::Claude, "claude-code"),
        };
        let mut config = (*self.config).clone();
        config.agent_profile = Some(profile.to_string());
        Self {
            kind,
            tmp: Arc::clone(&self.tmp),
            config: Arc::new(config),
        }
    }

    fn format(&self) -> Arc<dyn SessionFormat> {
        match self.kind {
            Kind::Claude => Arc::new(ClaudeSessionFormat),
            Kind::Codex => Arc::new(CodexSessionFormat),
        }
    }

    fn home(&self) -> PathBuf {
        PathBuf::from(match self.kind {
            Kind::Claude => &self.config.claude_config_dir,
            Kind::Codex => &self.config.codex_home_dir,
        })
    }

    fn cwd(&self) -> PathBuf {
        PathBuf::from(&self.config.workspace_dir)
    }

    fn workspace(&self) -> &Path {
        Path::new(&self.config.workspace_dir)
    }

    fn harness(&self, backend: &Arc<MemoryTrajectoryBackend>) -> HarnessRuntime {
        let backend: Arc<dyn BackendClient> = Arc::clone(backend) as Arc<dyn BackendClient>;
        let trajectory = Some(TrajectoryRecording::new(backend));
        match self.kind {
            Kind::Claude => HarnessRuntime::claude(ClaudeConfig {
                config: Arc::clone(&self.config),
                agent_state: Arc::new(AgentState::new()),
                agent_profile: Some("claude-code".to_string()),
                events: None,
                trajectory,
            }),
            Kind::Codex => HarnessRuntime::codex(CodexConfig {
                config: Arc::clone(&self.config),
                credential_failures: CredentialFailureDetector::default(),
                trajectory,
            }),
        }
    }

    fn task(&self, name: &str, backend: &Arc<MemoryTrajectoryBackend>) -> TestTask<'_> {
        TestTask {
            env: self,
            backend: Arc::clone(backend),
            id: name.to_string(),
            harness: self.harness(backend),
            session: None,
            fork: None,
            switch: false,
            turns: Vec::new(),
        }
    }

    /// A Task forked from `parent` at `cut`, as `task_forks` records it.
    fn fork_task(
        &self,
        name: &str,
        parent: &TestTask<'_>,
        cut: Cut,
        backend: &Arc<MemoryTrajectoryBackend>,
    ) -> TestTask<'_> {
        let (source_message_uuid, source_agent_turn_id) = match cut {
            Cut::Message(m) => (Some(m), None),
            Cut::Turn(t) => (None, Some(t)),
            Cut::End => (None, None),
        };
        let fork = ForkSource {
            source_session_id: parent.session(),
            source_message_uuid: source_message_uuid.clone(),
            source_agent_turn_id: source_agent_turn_id.clone(),
        };
        backend.set_fork(
            name,
            MemoryFork {
                source_session_id: parent.session(),
                source_message_uuid,
                source_agent_turn_id,
            },
        );
        let mut task = self.task(name, backend);
        task.fork = Some(fork);
        task
    }

    /// Every harness session file, so a resume or fork can only come from the
    /// trajectory.
    fn delete_all_session_files(&self) {
        let dirs = match self.kind {
            Kind::Claude => vec![self.home().join("projects")],
            Kind::Codex => vec![
                self.home().join("sessions"),
                self.home().join("archived_sessions"),
            ],
        };
        for dir in dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    async fn wait_for_no_process_in_workspace(&self) {
        let workspace = std::fs::canonicalize(self.workspace()).unwrap();
        for _ in 0..100 {
            let busy = std::fs::read_dir("/proc").unwrap().flatten().any(|entry| {
                entry.file_name().to_string_lossy().parse::<u32>().is_ok()
                    && std::fs::read_link(entry.path().join("cwd"))
                        .is_ok_and(|cwd| cwd == workspace)
            });
            if !busy {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("a harness process outlived its harness");
    }

    fn session_file(&self, id: &str) -> Option<PathBuf> {
        self.format().locate(&self.home(), &self.cwd(), id)
    }

    fn file_lines(&self, id: &str) -> Vec<String> {
        let path = self
            .session_file(id)
            .unwrap_or_else(|| panic!("no session file for {id}"));
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Ships lines a CLI wrote after it reported its result, as the next
    /// Turn's recorder would.
    async fn settle(&self, backend: &Arc<MemoryTrajectoryBackend>, task: &TestTask<'_>) {
        let backend: Arc<dyn BackendClient> = Arc::clone(backend) as Arc<dyn BackendClient>;
        let mut recorder = SessionRecorder::start(
            self.format(),
            TrajectoryRecording::new(backend),
            RecordTarget {
                task_id: task.id.clone(),
                harness_session_id: task.session(),
                home: self.home(),
                cwd: self.cwd(),
                subagent: None,
            },
            None,
        );
        recorder.flush_all().await.unwrap();
    }
}

/// Where a fork cuts its source, in the shape `task_forks` stores.
#[derive(Clone, Debug)]
enum Cut {
    Message(String),
    Turn(String),
    End,
}

#[derive(Debug, Default)]
struct TurnRecord {
    answer: String,
    agent_turn_id: Option<String>,
    blocks: Vec<Value>,
    outcome: Option<String>,
}

struct TestTask<'a> {
    env: &'a RealEnv,
    backend: Arc<MemoryTrajectoryBackend>,
    id: String,
    harness: HarnessRuntime,
    session: Option<String>,
    fork: Option<ForkSource>,
    /// The next Turn opens a switch session.
    switch: bool,
    turns: Vec<TurnRecord>,
}

impl<'a> TestTask<'a> {
    /// Run the next Turn on `env`'s harness, as a changed Agent profile does.
    async fn switch_to(&mut self, env: &'a RealEnv) {
        self.env = env;
        self.harness = env.harness(&self.backend);
        self.session = None;
        self.switch = true;
        env.wait_for_no_process_in_workspace().await;
    }

    fn session(&self) -> String {
        self.session.clone().expect("the Task has run a Turn")
    }

    fn input(&mut self, prompt: &str, extra_env: &[(&str, &str)]) -> Input {
        let mut env = HashMap::from([
            ("TASK_ID".to_string(), self.id.clone()),
            ("MAX_THINKING_TOKENS".to_string(), "2048".to_string()),
        ]);
        if self.env.kind == Kind::Claude {
            env.insert(
                "CLAUDE_CONFIG_DIR".to_string(),
                self.env.config.claude_config_dir.clone(),
            );
        }
        for (k, v) in extra_env {
            env.insert((*k).to_string(), (*v).to_string());
        }
        let fork = self.fork.take();
        let model = match self.env.kind {
            Kind::Claude => Some(
                std::env::var("CLAUDE_TEST_MODEL")
                    .unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string()),
            ),
            Kind::Codex => Some(std::env::var("CODEX_TEST_MODEL").unwrap_or_else(|_| {
                moatless_types::model::CodexModel::Gpt6Astra
                    .id()
                    .to_string()
            })),
        };
        let switch_harness = std::mem::take(&mut self.switch);
        Input {
            model,
            resume: self.session.clone().map(ResumeSession::WithId),
            start_fresh: fork.is_some() || switch_harness,
            fork,
            switch_harness,
            env: Some(env),
            ..Input::text(format!("{}-{}", self.id, self.turns.len() + 1), prompt)
        }
    }

    /// A new pod: the harness, and any app-server it holds, starts over.
    async fn restart(&mut self) {
        self.harness = self.env.harness(&self.backend);
        // The old app-server dies asynchronously, and while it lives Codex
        // refuses `thread/resume` with "already has an active writer".
        self.env.wait_for_no_process_in_workspace().await;
    }

    async fn turn(&mut self, prompt: &str) -> String {
        self.run(prompt, false).await;
        let record = self.turns.last().unwrap();
        assert_eq!(
            record.outcome.as_deref(),
            Some("completed"),
            "{} turn {} did not complete",
            self.id,
            self.turns.len()
        );
        eprintln!("[{}] {prompt}\n  -> {}", self.id, record.answer);
        record.answer.clone()
    }

    /// Runs `prompt` and cancels the Turn as soon as it calls a tool.
    async fn killed_turn(&mut self, prompt: &str) {
        self.run(prompt, true).await;
    }

    async fn run(&mut self, prompt: &str, cancel_on_tool: bool) {
        let input = self.input(prompt, &[]);
        self.harness.submit(input).await.unwrap();
        let mut record = TurnRecord::default();
        let signals = async {
            while let Some(signal) = self.harness.next_signal().await {
                match signal {
                    Signal::ExecutionStarted {
                        session,
                        agent_turn_id,
                        ..
                    } => {
                        self.session = Some(session.0);
                        record.agent_turn_id = agent_turn_id;
                    }
                    Signal::Source(event) => {
                        let block = block_json(&event);
                        // Only the assistant ever emits `text`/`tool_use`: the
                        // harness's own turns never feed a human chat message
                        // back in as a `text` user block, only tool results.
                        if block["type"] == "text" {
                            record.answer.push_str(block["text"].as_str().unwrap_or(""));
                            record.answer.push('\n');
                        }
                        if cancel_on_tool && block["type"] == "tool_use" {
                            self.harness.cancel();
                        }
                        record.blocks.push(block);
                    }
                    Signal::ExecutionFinished { outcome } => {
                        record.outcome = Some(match outcome {
                            Outcome::Completed { .. } => "completed".to_string(),
                            Outcome::Cancelled => "cancelled".to_string(),
                            Outcome::Failed(error) => format!("failed: {error:?}"),
                        });
                        return;
                    }
                    Signal::InputRejected { reason, .. } => {
                        record.outcome = Some(format!("rejected: {reason:?}"));
                        return;
                    }
                    _ => {}
                }
            }
        };
        tokio::time::timeout(TURN_TIMEOUT, signals)
            .await
            .expect("the Turn timed out");
        self.turns.push(record);
    }

    /// The cut just after Turn `n` (1-based), as the backend's fork row names it.
    ///
    /// Only supports the most recently run Turn: the Claude arm reads the
    /// backend's published events for the whole session, which only ends at
    /// Turn `n` while no later Turn has run yet.
    fn cut_after_turn(&self, n: usize) -> Cut {
        assert_eq!(
            n,
            self.turns.len(),
            "cut_after_turn must name the last Turn run"
        );
        let turn = &self.turns[n - 1];
        match self.env.kind {
            Kind::Claude => {
                let anchor = self
                    .backend
                    .events_on(&self.session())
                    .iter()
                    .rev()
                    .find_map(|e| {
                        (e.kind == "assistant")
                            .then(|| e.harness_item_id.clone())
                            .flatten()
                    })
                    .expect("the Turn published an assistant message");
                Cut::Message(anchor)
            }
            Kind::Codex => Cut::Turn(turn.agent_turn_id.clone().unwrap()),
        }
    }

    fn file_lines(&self) -> Vec<String> {
        self.env.file_lines(&self.session())
    }

    fn held_payloads(&self) -> Vec<String> {
        self.backend
            .events_on(&self.session())
            .into_iter()
            .map(|e| e.payload)
            .collect()
    }
}

fn knows(answer: &str, word: &str) -> bool {
    answer.to_uppercase().contains(word)
}

/// A block's wire shape, for assertions that check one field rather than
/// matching the whole [`CanonicalContentBlock`] enum.
fn block_json(block: &CanonicalContentBlock) -> Value {
    serde_json::to_value(block).expect("content block serializes")
}

fn assert_held_equals_file(task: &TestTask<'_>) {
    let events = task.backend.events_on(&task.session());
    let seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(
        seqs,
        (0..events.len() as i64).collect::<Vec<_>>(),
        "seqs are contiguous from 0"
    );
    assert_eq!(task.held_payloads(), task.file_lines());
}

// ── Record ───────────────────────────────────────────────────────────────────

async fn records_every_line(env: RealEnv) {
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut task = env.task("record", &backend);
    task.turn(
        "Our project's release name is PERIWINKLE. Use your shell tool to run `echo one > a.txt`, then reply ok.",
    )
    .await;
    task.turn("Use your shell tool to run `cat a.txt`, then reply with its output.")
        .await;
    env.settle(&backend, &task).await;
    assert_held_equals_file(&task);
    if env.kind == Kind::Claude {
        assert!(
            backend
                .events_on(&task.session())
                .iter()
                .any(|e| e.content.to_string().contains("\"type\":\"thinking\"")),
            "Claude ran with thinking on"
        );
    }
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn claude_records_every_line() {
    let Some(env) = RealEnv::claude() else { return };
    records_every_line(env).await;
}

#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn codex_records_every_line() {
    let Some(env) = RealEnv::codex() else { return };
    records_every_line(env).await;
}

// ── Rebuild ──────────────────────────────────────────────────────────────────

/// Covers `rebuilt_file_equals_the_original` and
/// `resume_records_only_new_lines` too.
async fn rebuilds_a_deleted_session(env: RealEnv) {
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut task = env.task("rebuild", &backend);
    task.turn(
        "Our project's release name is PERIWINKLE; keep it in mind. Use your shell tool to run `echo hi`, then reply ok.",
    )
    .await;
    env.settle(&backend, &task).await;
    let original = task.file_lines();
    assert_eq!(task.held_payloads(), original);

    env.delete_all_session_files();
    task.restart().await;
    let answer = task
        .turn("What is our project's release name? Reply with just the name.")
        .await;
    assert!(knows(&answer, "PERIWINKLE"), "{answer}");

    // rebuilt_file_equals_the_original: the rebuilt prefix is the file as it
    // was, the resume id being the recorded one.
    let rebuilt = task.file_lines();
    assert_eq!(rebuilt[..original.len()], original[..]);
    // resume_records_only_new_lines
    env.settle(&backend, &task).await;
    assert_eq!(backend.stored_event_count(), rebuilt.len());
    assert_held_equals_file(&task);
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn claude_rebuilds_a_deleted_session() {
    let Some(env) = RealEnv::claude() else { return };
    rebuilds_a_deleted_session(env).await;
}

#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn codex_rebuilds_a_deleted_session() {
    let Some(env) = RealEnv::codex() else { return };
    rebuilds_a_deleted_session(env).await;
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn thinking_survives_rebuild() {
    let Some(env) = RealEnv::claude() else { return };
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut task = env.task("thinking", &backend);
    task.turn(
        "Think it through step by step: what is 17 * 23? Our project's release name is SAFFRON. Reply with the product only.",
    )
    .await;
    let signed = backend.events_on(&task.session()).iter().any(|e| {
        e.content["blocks"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|b| {
                b["type"] == "thinking" && b["signature"].as_str().is_some_and(|s| !s.is_empty())
            })
    });
    assert!(signed, "the session holds signed thinking");

    env.delete_all_session_files();
    task.restart().await;
    let answer = task
        .turn(
            "Think briefly, then tell me our project's release name and the product you computed.",
        )
        .await;
    assert!(knows(&answer, "SAFFRON"), "{answer}");
    assert!(answer.contains("391"), "{answer}");
}

// ── Switch ───────────────────────────────────────────────────────────────────

/// Claude → Codex → Claude: the second Claude session forks the first
/// verbatim and renders only the Codex turn after it (D8).
#[tokio::test]
#[ignore = "runs real Claude Code and Codex; needs TRAJECTORY_E2E=1"]
async fn switch_back_resumes_verbatim() {
    let Some(claude) = RealEnv::claude() else {
        return;
    };
    let codex = claude.other();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut task = claude.task("switching", &backend);
    task.turn("Our project's release name is PERIWINKLE; keep it in mind. Reply ok.")
        .await;
    claude.settle(&backend, &task).await;
    let first = task.session();
    let first_events = backend.events_on(&first);

    task.switch_to(&codex).await;
    let answer = task
        .turn("Our project's second release name is SAFFRON; keep it in mind. What was the first one? Reply with just that name.")
        .await;
    assert!(knows(&answer, "PERIWINKLE"), "{answer}");
    codex.settle(&backend, &task).await;
    let codex_session = task.session();

    claude.delete_all_session_files();
    task.switch_to(&claude).await;
    let answer = task
        .turn("List every release name I have told you, or reply NONE.")
        .await;
    assert!(
        knows(&answer, "PERIWINKLE") && knows(&answer, "SAFFRON"),
        "{answer}"
    );
    claude.settle(&backend, &task).await;
    let back = task.session();
    assert_ne!(back, first);
    assert_ne!(back, codex_session);
    let held: Vec<String> = backend
        .events_on(&back)
        .into_iter()
        .map(|e| e.payload)
        .collect();
    let verbatim: Vec<String> = first_events.into_iter().map(|e| e.payload).collect();
    assert_eq!(
        held[..verbatim.len()],
        verbatim[..],
        "the first session is forked verbatim"
    );
    assert!(
        held.len() > verbatim.len(),
        "the Codex turn and this one follow"
    );
    let file = task.file_lines();
    assert_eq!(
        file.len(),
        held.len(),
        "the file is what the trajectory holds"
    );
    // PERIWINKLE's prompt appears once: Turn 1 was not rendered again.
    let periwinkle = file
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|l| l["type"] == "user" && l["message"].to_string().contains("PERIWINKLE; keep it"))
        .count();
    assert_eq!(periwinkle, 1, "Turn 1 is not transcoded back");
}

// ── Fork ─────────────────────────────────────────────────────────────────────

async fn fork_knows_only_the_prefix(env: RealEnv) {
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut parent = env.task("parent", &backend);
    parent
        .turn("Our project's release name is PERIWINKLE; keep it in mind. Use your shell tool to run `echo one > a.txt`, then reply ok.")
        .await;
    let cut = parent.cut_after_turn(1);
    parent
        .turn("Our project's second release name is SAFFRON; keep it in mind. Reply ok.")
        .await;
    env.settle(&backend, &parent).await;
    let stored = backend.stored_event_count();

    env.delete_all_session_files();
    let mut child = env.fork_task("child", &parent, cut, &backend);
    let answer = child
        .turn("List every release name I have told you, or reply NONE.")
        .await;
    assert!(knows(&answer, "PERIWINKLE"), "{answer}");
    assert!(!knows(&answer, "SAFFRON"), "{answer}");

    env.settle(&backend, &child).await;
    let own: Vec<String> = backend
        .own_events(&child.session())
        .into_iter()
        .map(|e| e.payload)
        .collect();
    assert_eq!(
        backend.stored_event_count(),
        stored + own.len(),
        "no event is copied"
    );
    let file = child.file_lines();
    assert_eq!(file.len(), child.held_payloads().len());
    let prefix = file.len() - own.len();
    assert!(prefix > 0, "the prefix is the parent's");
    assert_eq!(file[prefix..], own[..]);

    let answer = parent
        .turn("List every release name I have told you, or reply NONE.")
        .await;
    assert!(
        knows(&answer, "PERIWINKLE") && knows(&answer, "SAFFRON"),
        "{answer}"
    );
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn claude_fork_knows_only_the_prefix() {
    let Some(env) = RealEnv::claude() else { return };
    fork_knows_only_the_prefix(env).await;
}

#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn codex_fork_knows_only_the_prefix() {
    let Some(env) = RealEnv::codex() else { return };
    fork_knows_only_the_prefix(env).await;
}

async fn fork_right_after_a_turn_sees_the_turn(env: RealEnv) {
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut parent = env.task("parent", &backend);
    parent
        .turn("Our project's release name is MARIGOLD; keep it in mind. Reply ok.")
        .await;
    // No settle: the fork reads what the Turn flushed before it finished.
    env.delete_all_session_files();
    let mut child = env.fork_task("child", &parent, Cut::End, &backend);
    let answer = child
        .turn("What is our project's release name? Reply with just the name.")
        .await;
    assert!(knows(&answer, "MARIGOLD"), "{answer}");
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn claude_fork_right_after_a_turn_sees_the_turn() {
    let Some(env) = RealEnv::claude() else { return };
    fork_right_after_a_turn_sees_the_turn(env).await;
}

#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn codex_fork_right_after_a_turn_sees_the_turn() {
    let Some(env) = RealEnv::codex() else { return };
    fork_right_after_a_turn_sees_the_turn(env).await;
}

/// A cut on the `tool_use` line keeps its result, so the fork resumes a
/// well-formed history.
#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn fork_at_a_tool_use_includes_its_result() {
    let Some(env) = RealEnv::claude() else { return };
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut parent = env.task("parent", &backend);
    parent
        .turn("Use your shell tool to run `echo CERULEAN`, then reply with its output.")
        .await;
    parent
        .turn("Our project's release name is SAFFRON; keep it in mind. Reply ok.")
        .await;
    // Only the assistant ever emits a `tool_use` block, so the block type
    // alone identifies it without needing the block's originating role.
    let tool_use = parent.turns[0]
        .blocks
        .iter()
        .position(|b| b["type"] == "tool_use")
        .expect("turn 1 called a tool");
    let events = backend.events_on(&parent.session());
    let tool_use_id = parent.turns[0].blocks[tool_use]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let anchor = events
        .iter()
        .find(|e| e.payload.contains(&tool_use_id) && e.kind == "assistant")
        .and_then(|e| e.harness_item_id.clone())
        .unwrap();

    env.delete_all_session_files();
    let mut child = env.fork_task("child", &parent, Cut::Message(anchor), &backend);
    let answer = child
        .turn("What did the shell command you ran print? And what is our project's release name, or NONE?")
        .await;
    assert!(knows(&answer, "CERULEAN"), "{answer}");
    assert!(!knows(&answer, "SAFFRON"), "{answer}");
    let child_lines = child.file_lines();
    assert!(
        child_lines
            .iter()
            .any(|l| l.contains(&tool_use_id) && l.contains("tool_result")),
        "the fork holds the tool's result"
    );
}

async fn fork_of_a_fork_knows_both_prefixes(env: RealEnv) {
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut root = env.task("root", &backend);
    root.turn("Our project's first release name is PERIWINKLE; keep it in mind. Reply ok.")
        .await;
    let root_cut = root.cut_after_turn(1);
    root.turn("Our project's unused release name is CRIMSON; keep it in mind. Reply ok.")
        .await;

    let mut child = env.fork_task("child", &root, root_cut, &backend);
    child
        .turn("Our project's second release name is SAFFRON; keep it in mind. Reply ok.")
        .await;
    let child_cut = child.cut_after_turn(1);
    child
        .turn("Our project's other unused release name is OCHRE; keep it in mind. Reply ok.")
        .await;

    env.delete_all_session_files();
    let mut grandchild = env.fork_task("grandchild", &child, child_cut, &backend);
    let answer = grandchild
        .turn("List every release name I have told you, or reply NONE.")
        .await;
    assert!(knows(&answer, "PERIWINKLE"), "{answer}");
    assert!(knows(&answer, "SAFFRON"), "{answer}");
    assert!(!knows(&answer, "CRIMSON"), "{answer}");
    assert!(!knows(&answer, "OCHRE"), "{answer}");
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn claude_fork_of_a_fork_knows_both_prefixes() {
    let Some(env) = RealEnv::claude() else { return };
    fork_of_a_fork_knows_both_prefixes(env).await;
}

#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn codex_fork_of_a_fork_knows_both_prefixes() {
    let Some(env) = RealEnv::codex() else { return };
    fork_of_a_fork_knows_both_prefixes(env).await;
}

// ── Crash ────────────────────────────────────────────────────────────────────

async fn resume_after_a_killed_turn(env: RealEnv) {
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut task = env.task("killed", &backend);
    task.turn("Our project's release name is PERIWINKLE; keep it in mind. Reply ok.")
        .await;
    task.killed_turn("Use your shell tool to run `sleep 60 && echo late`, then reply done.")
        .await;
    let outcome = task.turns.last().unwrap().outcome.clone();
    eprintln!("killed Turn ended: {outcome:?}");

    env.delete_all_session_files();
    task.restart().await;
    let answer = task
        .turn("The last command was interrupted; do not rerun it. What is our project's release name? Reply with just the name.")
        .await;
    assert!(knows(&answer, "PERIWINKLE"), "{answer}");
    env.settle(&backend, &task).await;
    assert_held_equals_file(&task);
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn claude_resume_after_a_killed_turn() {
    let Some(env) = RealEnv::claude() else { return };
    resume_after_a_killed_turn(env).await;
}

#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn codex_resume_after_a_killed_turn() {
    let Some(env) = RealEnv::codex() else { return };
    resume_after_a_killed_turn(env).await;
}

// ── Tool results ─────────────────────────────────────────────────────────────

/// `tool-results/` rides in transcripts.tar, not the trajectory: the test
/// deletes everything, restores that directory as the archive would, and
/// rebuilds the session file from the trajectory.
#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn claude_rebuilt_session_reads_a_persisted_tool_output() {
    let Some(env) = RealEnv::claude() else { return };
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut big = String::new();
    for i in 0..6000 {
        writeln!(big, "filler line {i:05} lorem ipsum dolor sit amet").unwrap();
    }
    big.push_str("FINAL TOKEN: VERMILION\n");
    std::fs::write(env.workspace().join("big.txt"), big).unwrap();

    let mut task = env.task("tool-results", &backend);
    task.turn(
        "Use your shell tool to run `cat big.txt` and reply only ok. Do not read the output.",
    )
    .await;
    std::fs::remove_file(env.workspace().join("big.txt")).unwrap();

    let session_dir = env
        .session_file(&task.session())
        .unwrap()
        .with_extension("");
    let persisted = session_dir.join("tool-results");
    assert!(
        std::fs::read_dir(&persisted).is_ok_and(|mut d| d.next().is_some()),
        "the CLI persisted the output under {}",
        persisted.display()
    );
    let archive = env.tmp.path().join("archive");
    copy_dir(&session_dir, &archive);

    env.delete_all_session_files();
    copy_dir(&archive, &session_dir);
    task.restart().await;
    let answer = task
        .turn("big.txt is gone. Read the saved output of that cat command and tell me the FINAL TOKEN on its last line.")
        .await;
    assert!(knows(&answer, "VERMILION"), "{answer}");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

// ── Content ──────────────────────────────────────────────────────────────────

/// The `(role, block type)` sequence a trajectory's content shows, without
/// thinking and with runs of the same pair collapsed.
fn content_shape(backend: &MemoryTrajectoryBackend, session: &str) -> Vec<(String, String)> {
    let mut shape: Vec<(String, String)> = Vec::new();
    // A code-mode command's own call and result sit under its `exec` call.
    let events = backend.events_on(session);
    for event in events
        .iter()
        .filter(|e| e.content.get("derived_from").is_none())
    {
        let role = event.content["role"].as_str().unwrap_or("").to_string();
        for block in event.content["blocks"].as_array().into_iter().flatten() {
            let kind = block["type"].as_str().unwrap_or("").to_string();
            // Codex records its instructions as a `system` message.
            if kind == "thinking" || role.is_empty() || role == "system" {
                continue;
            }
            let pair = (role.clone(), kind);
            if shape.last() != Some(&pair) {
                shape.push(pair);
            }
        }
    }
    shape
}

/// The prompt is typed, not injected; a response carries its usage; nothing
/// compacted. Compaction lines are covered by the recorded fixtures.
fn assert_classified(kind: Kind, events: &[memory::MemoryEvent], prompt: &str) {
    let typed: Vec<_> = events
        .iter()
        .filter(|e| e.content["blocks"][0]["text"] == prompt)
        .collect();
    assert_eq!(typed.len(), 1, "{kind:?} records the prompt once");
    assert_eq!(typed[0].content["injected"], false);
    assert!(events.iter().all(|e| e.content["compaction"] == false));
    assert!(
        events
            .iter()
            .any(|e| e.content["usage"]["output_tokens"].as_u64() > Some(0)),
        "{kind:?} records a response's usage"
    );
    if kind == Kind::Codex {
        assert!(
            events.iter().any(|e| e.content["injected"] == true),
            "Codex writes its environment context as a user message"
        );
    }
}

#[tokio::test]
#[ignore = "runs real Claude Code and Codex; needs TRAJECTORY_E2E=1"]
async fn both_harnesses_give_one_content_shape() {
    let (Some(claude), Some(codex)) = (RealEnv::claude(), RealEnv::codex()) else {
        return;
    };
    let prompt =
        "Use your shell tool to run `echo shape`. After it returns, reply with just its output.";
    let mut shapes = Vec::new();
    for env in [&claude, &codex] {
        let backend = Arc::new(MemoryTrajectoryBackend::default());
        let mut task = env.task("shape", &backend);
        task.turn(prompt).await;
        env.settle(&backend, &task).await;
        let shape = content_shape(&backend, &task.session());
        eprintln!("{:?}: {shape:?}", env.kind);
        shapes.push(shape);
        assert_classified(env.kind, &backend.events_on(&task.session()), prompt);
    }
    let pair = |r: &str, k: &str| (r.to_string(), k.to_string());
    let expected = vec![
        pair("user", "text"),
        pair("assistant", "tool_use"),
        pair("user", "tool_result"),
        pair("assistant", "text"),
    ];
    for shape in &shapes {
        // A harness may open with a line of assistant text before its call.
        let trimmed: Vec<_> = shape
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                !(p.0 == "assistant"
                    && p.1 == "text"
                    && shape.get(i + 1).is_some_and(|n| n.1 == "tool_use"))
            })
            .map(|(_, p)| p.clone())
            .collect();
        assert_eq!(trimmed, expected);
    }
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn claude_records_a_subagent_session() {
    let Some(env) = RealEnv::claude() else {
        return;
    };
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut task = env.task("subagent", &backend);
    task.turn(
        "Use your Agent tool to launch one general-purpose subagent with the prompt \
         `Use your shell tool to run echo SUBMARINE, then reply with its output.` \
         Then reply with what the subagent said.",
    )
    .await;
    let session = task.session();
    let subagents: Vec<String> = backend
        .session_ids()
        .into_iter()
        .filter(|id| id.starts_with(&format!("{session}:agent-")))
        .collect();
    assert_eq!(subagents.len(), 1, "{:?}", backend.session_ids());
    let (parent, seq) = backend.parent_of(&subagents[0]).expect("linked");
    assert_eq!(parent, session);
    let spawning = &backend.events_on(&session)[usize::try_from(seq).unwrap()];
    assert!(spawning.content.to_string().contains("\"tool_use\""));
    let lines = backend.events_on(&subagents[0]);
    assert!(lines.iter().any(|e| e.payload.contains("SUBMARINE")));
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn lines_ship_before_the_turn_ends() {
    let Some(env) = RealEnv::claude() else {
        return;
    };
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let mut task = env.task("live", &backend);
    let done = std::sync::atomic::AtomicBool::new(false);
    let watched = Arc::clone(&backend);
    let saw_call_early = async {
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            let early = watched.session_ids().iter().any(|id| {
                watched
                    .own_events(id)
                    .iter()
                    .any(|e| e.content.to_string().contains("\"tool_use\""))
            });
            if early {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    };
    let turn = async {
        task.turn("Use your shell tool to run `sleep 6 && echo done`, then reply with its output.")
            .await;
        done.store(true, std::sync::atomic::Ordering::SeqCst);
    };
    let (early, ()) = tokio::join!(saw_call_early, turn);
    assert!(
        early,
        "the tool call reached the backend while the command ran"
    );
    assert!(backend.widest_append() <= 256);
}

// ── Transcode (phase 0) ──────────────────────────────────────────────────────

/// One recorded conversation in each harness, rendered into the other
/// harness's format, as `SessionFormat::render` renders them.
const CODEX_FROM_CLAUDE: &str = include_str!("fixtures/transcode/codex-from-claude.jsonl");
const CLAUDE_FROM_CODEX: &str = include_str!("fixtures/transcode/claude-from-codex.jsonl");
const CODEX_FROM_CLAUDE_COMPACTED: &str =
    include_str!("fixtures/transcode/codex-from-claude-compacted.jsonl");
const CLAUDE_FROM_CODEX_COMPACTED: &str =
    include_str!("fixtures/transcode/claude-from-codex-compacted.jsonl");

impl RealEnv {
    /// Writes a transcoded fixture as a session file of this harness under a
    /// new id, as a switch session would be written, and returns the id.
    fn install_transcoded(&self, fixture: &str) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        let cwd = std::fs::canonicalize(self.cwd()).unwrap();
        let body = fixture
            .replace("{{SESSION_ID}}", &id)
            .replace("{{CWD}}", &cwd.display().to_string());
        let first = body.lines().next().unwrap();
        let path = self.format().session_path(&self.home(), &cwd, &id, first);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        id
    }
}

impl TestTask<'_> {
    /// Resumes a transcoded session the Task did not write.
    fn resuming(mut self, session: String) -> Self {
        self.session = Some(session);
        self
    }
}

async fn transcoded_resume_knows_history(env: RealEnv, fixture: &str) {
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let session = env.install_transcoded(fixture);
    let mut task = env.task("transcoded", &backend).resuming(session.clone());
    let answer = task
        .turn("Without using any tools: what is our project's release name, and which file did we write it into? Reply in one sentence.")
        .await;
    assert!(knows(&answer, "PERIWINKLE"), "{answer}");
    assert!(answer.contains("release.txt"), "{answer}");
    assert_eq!(
        task.session(),
        session,
        "the CLI resumed the synthesised session"
    );
}

#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn claude_to_codex_resume_knows_history() {
    let Some(env) = RealEnv::codex() else { return };
    transcoded_resume_knows_history(env, CODEX_FROM_CLAUDE).await;
}

#[tokio::test]
#[ignore = "runs real Claude Code; needs TRAJECTORY_E2E=1"]
async fn codex_to_claude_resume_knows_history() {
    let Some(env) = RealEnv::claude() else { return };
    transcoded_resume_knows_history(env, CLAUDE_FROM_CODEX).await;
}

const CODEX_RECORDED: &str = include_str!("fixtures/transcode/codex.jsonl");
const CODEX_RECORDED_ID: &str = "01a0de79-4dff-72c0-a0de-c819e502d676";

/// A switch back to Codex appends rendered lines to the rollout Codex wrote
/// itself, whose every line carries an `ordinal`.
#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn codex_resumes_its_own_rollout_with_rendered_lines_appended() {
    let Some(env) = RealEnv::codex() else { return };
    let native = CODEX_RECORDED
        .replace(CODEX_RECORDED_ID, "{{SESSION_ID}}")
        .replace("/workspace", "{{CWD}}");
    let prior: Vec<&str> = native.lines().take(38).collect();
    let text = |role, text: &str| EventContent {
        role: Some(role),
        blocks: vec![CanonicalContentBlock::Text { text: text.into() }],
        ..EventContent::default()
    };
    let rendered = CodexSessionFormat.render(
        &[
            text(
                Role::User,
                "Our second release is named OCHRE-4471. Reply OK.",
            ),
            text(Role::Assistant, "OK."),
        ],
        &RenderContext {
            harness_session_id: "{{SESSION_ID}}",
            cwd: Path::new("{{CWD}}"),
            model: None,
            prior: &prior,
            timestamp: "2026-09-26T16:09:00.000Z",
        },
    );
    let mut body = prior.join("\n");
    for line in &rendered {
        body.push('\n');
        body.push_str(line);
    }
    body.push('\n');

    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let session = env.install_transcoded(&body);
    let mut task = env.task("appended", &backend).resuming(session.clone());
    let answer = task
        .turn(
            "Without using any tools: what is our second release named? Reply with the name only.",
        )
        .await;
    assert!(answer.contains("4471"), "{answer}");
    assert_eq!(
        task.session(),
        session,
        "the CLI resumed the appended rollout"
    );
}

/// Claude's `Read` becomes a Codex `exec_command`, and Codex's `apply_patch`
/// a Claude `Edit`. The test writes 4471 into that call's result, and nowhere
/// else, so only the mapped call's result can supply it.
#[tokio::test]
#[ignore = "runs real Claude Code and Codex; needs TRAJECTORY_E2E=1"]
async fn transcoded_tool_call_result_is_seen() {
    let (Some(claude), Some(codex)) = (RealEnv::claude(), RealEnv::codex()) else {
        return;
    };
    let backend = Arc::new(MemoryTrajectoryBackend::default());

    let fixture = with_result_marker(
        CODEX_FROM_CLAUDE,
        r#""output":"1\tPERIWINKLE""#,
        r#""output":"1\tPERIWINKLE\n2\tchecksum 4471""#,
    );
    let session = codex.install_transcoded(&fixture);
    let mut task = codex.task("mapped", &backend).resuming(session);
    let answer = task
        .turn("Without using any tools: when release.txt was read earlier in this conversation, what did its second line say? Reply in one sentence.")
        .await;
    assert!(answer.contains("4471"), "the mapped Read result: {answer}");

    let fixture = with_result_marker(
        CLAUDE_FROM_CODEX,
        "Wall time: 0 seconds",
        "Wall time: 4471 seconds",
    );
    let session = claude.install_transcoded(&fixture);
    let mut task = claude.task("mapped", &backend).resuming(session);
    let answer = task
        .turn("Without using any tools: what wall time did the edit of release.txt earlier in this conversation report? Reply in one sentence.")
        .await;
    assert!(
        answer.contains("4471"),
        "the mapped apply_patch result: {answer}"
    );
}

/// A call the other harness has no tool for becomes a described call, whose
/// text alone carries its result.
#[tokio::test]
#[ignore = "runs real Codex; needs TRAJECTORY_E2E=1"]
async fn transcoded_tool_call_is_described() {
    let Some(codex) = RealEnv::codex() else {
        return;
    };
    let prior: Vec<&str> = CODEX_FROM_CLAUDE.lines().collect();
    let items = [
        EventContent {
            role: Some(Role::Assistant),
            blocks: vec![CanonicalContentBlock::ToolUse {
                id: "toolu_fetch".into(),
                name: "WebFetch".into(),
                input: serde_json::json!({"url": "https://example.com/build"}),
            }],
            ..EventContent::default()
        },
        EventContent {
            role: Some(Role::User),
            blocks: vec![CanonicalContentBlock::ToolResult {
                tool_use_id: "toolu_fetch".into(),
                content: Some(Value::from("The page says the build code is 4471.")),
                is_error: None,
            }],
            ..EventContent::default()
        },
    ];
    let rendered = CodexSessionFormat.render(
        &items,
        &RenderContext {
            harness_session_id: "{{SESSION_ID}}",
            cwd: Path::new("{{CWD}}"),
            model: None,
            prior: &prior,
            timestamp: "2026-09-26T16:09:00.000Z",
        },
    );
    assert!(rendered[0].contains("Tool call `WebFetch`"), "{rendered:?}");
    let body = format!("{}{}\n", CODEX_FROM_CLAUDE, rendered.join("\n"));

    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let session = codex.install_transcoded(&body);
    let mut task = codex.task("described", &backend).resuming(session);
    let answer = task
        .turn("Without using any tools: what build code did the web page fetched earlier in this conversation give? Reply in one sentence.")
        .await;
    assert!(
        answer.contains("4471"),
        "the described WebFetch result: {answer}"
    );
}

/// `fixture` with the one occurrence of `from` replaced by `to`.
fn with_result_marker(fixture: &str, from: &str, to: &str) -> String {
    assert_eq!(
        fixture.matches(from).count(),
        1,
        "{from} is in the fixture once"
    );
    fixture.replace(from, to)
}

/// S5: a compacted session transcodes from its summary.
#[tokio::test]
#[ignore = "runs real Claude Code and Codex; needs TRAJECTORY_E2E=1"]
async fn transcoded_compaction_keeps_the_summary() {
    let (Some(claude), Some(codex)) = (RealEnv::claude(), RealEnv::codex()) else {
        return;
    };
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    for (env, fixture) in [
        (&codex, CODEX_FROM_CLAUDE_COMPACTED),
        (&claude, CLAUDE_FROM_CODEX_COMPACTED),
    ] {
        let session = env.install_transcoded(fixture);
        let mut task = env.task("compacted", &backend).resuming(session);
        let answer = task
            .turn("Without using any tools: list every release name I have told you, separated by commas.")
            .await;
        assert!(
            knows(&answer, "PERIWINKLE") && knows(&answer, "SAFFRON"),
            "{:?}: {answer}",
            env.kind
        );
    }
}
