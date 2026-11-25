//! What every E2E test shares: the `Backend` capability matrix, the two
//! environment gates, and the option builders and message collectors.

use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::ContentBlock;
use claude_agent_sdk::QueryHandle;
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::api::content::ApiMessage;
use claude_agent_sdk::api::messages::{MessagePriority, ResultMessage, UserMessage};
use claude_agent_sdk::query;
use claude_agent_sdk::query_streaming;
use claude_agent_sdk::query_tmux;
use claude_agent_sdk::query_tmux_streaming;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

/// Default test timeout — generous because CLI startup + API call can be slow.
pub(crate) const TEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Longer timeout for multi-turn / session-resume / subagent flows.
pub(crate) const LONG_TEST_TIMEOUT: Duration = Duration::from_secs(90);

/// Even longer timeout for tmux subagent tests — spawning a sub-claude inside a
/// TUI adds noticeable latency.
pub(crate) const TMUX_LONG_TEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Cheapest, fastest model for E2E tests.
pub(crate) const TEST_MODEL: &str = "claude-haiku-4-5-20251001";

/// Which Claude backend a test runs against.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Backend {
    Subprocess,
    Tmux,
}

impl Backend {
    pub(crate) async fn start_one_shot(
        self,
        prompt: &str,
        options: ClaudeOptions,
    ) -> claude_agent_sdk::Result<QueryHandle> {
        match self {
            Self::Subprocess => query(prompt, options).await,
            Self::Tmux => query_tmux(prompt, options).await,
        }
    }

    pub(crate) async fn start_streaming(
        self,
        options: ClaudeOptions,
    ) -> claude_agent_sdk::Result<QueryHandle> {
        match self {
            Self::Subprocess => query_streaming(options).await,
            Self::Tmux => query_tmux_streaming(options).await,
        }
    }

    /// `Result.total_cost_usd` is real for subprocess, synthesized 0.0 for
    /// tmux.
    pub(crate) fn has_cost_data(self) -> bool {
        matches!(self, Self::Subprocess)
    }

    /// Whether the message channel stays open after `SdkMessage::Result`. Tmux
    /// yes (TUI keeps running), subprocess no (CLI exits in --print mode).
    pub(crate) fn channel_stays_open_after_result(self) -> bool {
        matches!(self, Self::Tmux)
    }
}

pub(crate) fn tmux_is_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok()
}

/// Initialize tracing for test output (idempotent).
pub(crate) fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("claude_agent_sdk=debug".parse().unwrap()),
        )
        .with_test_writer()
        .try_init();
}

/// Collect auth-related env vars to forward explicitly to the CLI subprocess.
pub(crate) fn auth_env() -> Option<HashMap<String, String>> {
    let mut env = HashMap::new();
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        env.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token);
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env.insert("ANTHROPIC_API_KEY".to_string(), key);
    }
    if env.is_empty() { None } else { Some(env) }
}

/// Default options for E2E tests: haiku model, 1 turn, skip permissions.
pub(crate) fn e2e_options() -> ClaudeOptions {
    e2e_options_with_cwd(None)
}

/// Same as `e2e_options` but with an explicit `cwd`. Tmux tests need a fresh
/// per-test cwd so the JSONL discovery snapshot-diff in
/// `~/.claude/projects/<dashed-cwd>/` doesn't race with concurrent tests.
pub(crate) fn e2e_options_with_cwd(cwd: Option<&Path>) -> ClaudeOptions {
    ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(1),
        allow_dangerously_skip_permissions: Some(true),
        cwd: cwd.map(|p| p.to_string_lossy().into_owned()),
        env: auth_env(),
        ..Default::default()
    }
}

/// Allocates a fresh, unique directory under `std::env::temp_dir()` for a
/// test's `cwd`.
///
/// The tmux backend's watcher dismisses the workspace-trust dialog for an
/// unfamiliar cwd, and the Stop hook still reports the real transcript path.
pub(crate) fn fresh_test_cwd(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join("claude-sdk-e2e");
    std::fs::create_dir_all(&root).expect("create e2e root");
    let cwd = root.join(format!("{name}-{}", uuid::Uuid::new_v4().simple()));
    let _ = std::fs::remove_dir_all(&cwd);
    std::fs::create_dir_all(&cwd).expect("create fresh test cwd");
    cwd
}

/// Collect all messages from a query handle until Result or channel close.
pub(crate) async fn collect_messages(
    rx: &mut tokio::sync::mpsc::Receiver<claude_agent_sdk::Result<SdkMessage>>,
) -> Vec<SdkMessage> {
    let mut messages = Vec::new();
    while let Some(msg) = rx.recv().await {
        match msg {
            Ok(msg) => {
                let is_result = matches!(&msg, SdkMessage::Result(_));
                messages.push(msg);
                if is_result {
                    break;
                }
            }
            Err(e) => {
                eprintln!("Error receiving message: {e}");
                break;
            }
        }
    }
    messages
}

/// Extract all text from assistant messages.
pub(crate) fn extract_assistant_text(messages: &[SdkMessage]) -> String {
    let mut text = String::new();
    for msg in messages {
        if let SdkMessage::Assistant(assistant) = msg {
            for block in &assistant.message.content {
                if let ContentBlock::Text { text: t } = block {
                    text.push_str(t);
                }
            }
        }
    }
    text
}

/// Extract the result text from a successful Result message.
pub(crate) fn extract_result_text(messages: &[SdkMessage]) -> Option<String> {
    messages.iter().find_map(|m| {
        if let SdkMessage::Result(ResultMessage::Success(s)) = m {
            Some(s.result.clone())
        } else {
            None
        }
    })
}

/// Send `text` as a user turn on a streaming handle.
///
/// `session_id` comes from the first turn's `SdkMessage::System`; pass `""`
/// before one has arrived.
pub(crate) async fn send_text(
    handle: &QueryHandle,
    text: &str,
    session_id: &str,
) -> claude_agent_sdk::Result<()> {
    let message = UserMessage {
        uuid: None,
        session_id: session_id.to_string(),
        message: ApiMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            usage: None,
            model: None,
            id: None,
            stop_reason: None,
            stop_sequence: None,
        },
        parent_tool_use_id: None,
        is_synthetic: None,
        tool_use_result: None,
        is_meta: None,
        is_compact_summary: None,
        is_visible_in_transcript_only: None,
        plan_content: None,
        thinking_metadata: None,
        todos: None,
        summarize_metadata: None,
        source_tool_assistant_uuid: None,
        source_tool_use_id: None,
        permission_mode: None,
        priority: MessagePriority::Next,
    };
    handle.send_user_message(&message).await
}
