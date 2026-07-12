use std::collections::HashMap;
use std::process::Stdio;

use serde::de::{Deserializer, Error as DeError};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;

use super::options::build_prompt_append;
use super::types::{AgentRunRequest, ResumeSession};

pub struct CodexCliSession {
    pub messages: mpsc::UnboundedReceiver<Result<CodexExecEvent, String>>,
    cancel_token: CancellationToken,
}

impl CodexCliSession {
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum CodexExecEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted(ThreadStartedEvent),
    #[serde(rename = "turn.started")]
    TurnStarted(TurnStartedEvent),
    #[serde(rename = "turn.completed")]
    TurnCompleted(TurnCompletedEvent),
    #[serde(rename = "turn.failed")]
    TurnFailed(TurnFailedEvent),
    #[serde(rename = "item.started")]
    ItemStarted(ItemEvent),
    #[serde(rename = "item.updated")]
    ItemUpdated(ItemEvent),
    #[serde(rename = "item.completed")]
    ItemCompleted(ItemEvent),
    #[serde(rename = "error")]
    Error(ThreadErrorEvent),
}

#[derive(Debug, Deserialize)]
pub struct ThreadStartedEvent {
    pub thread_id: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct TurnStartedEvent {}

#[derive(Debug, Deserialize)]
pub struct TurnCompletedEvent {
    pub usage: Usage,
}

#[derive(Debug, Deserialize)]
pub struct TurnFailedEvent {
    pub error: ThreadErrorEvent,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Debug, Deserialize)]
pub struct ItemEvent {
    pub item: ThreadItem,
}

#[derive(Debug, Deserialize)]
pub struct ThreadErrorEvent {
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ThreadItem {
    pub id: String,
    #[serde(flatten)]
    pub details: ThreadItemDetails,
}

/// Typed payloads for each supported codex thread item type.
///
/// Mirrors the upstream `ThreadItemDetails` enum at
/// `codex-rs/exec/src/exec_events.rs`. Unknown item types (introduced by
/// newer codex releases) land in `Other` with the raw JSON retained so the
/// processing loop can log them instead of silently dropping.
#[derive(Debug)]
pub enum ThreadItemDetails {
    AgentMessage(AgentMessageItem),
    Reasoning(ReasoningItem),
    CommandExecution(CommandExecutionItem),
    FileChange(FileChangeItem),
    McpToolCall(McpToolCallItem),
    CollabToolCall(CollabToolCallItem),
    WebSearch(WebSearchItem),
    TodoList(TodoListItem),
    Error(ErrorItem),
    Other {
        type_name: String,
        raw: serde_json::Value,
    },
}

impl<'de> Deserialize<'de> for ThreadItemDetails {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let type_name = value
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        match type_name.as_str() {
            "agent_message" => serde_json::from_value(value)
                .map(Self::AgentMessage)
                .map_err(D::Error::custom),
            "reasoning" => serde_json::from_value(value)
                .map(Self::Reasoning)
                .map_err(D::Error::custom),
            "command_execution" => serde_json::from_value(value)
                .map(Self::CommandExecution)
                .map_err(D::Error::custom),
            "file_change" => serde_json::from_value(value)
                .map(Self::FileChange)
                .map_err(D::Error::custom),
            "mcp_tool_call" => serde_json::from_value(value)
                .map(Self::McpToolCall)
                .map_err(D::Error::custom),
            "collab_tool_call" => serde_json::from_value(value)
                .map(Self::CollabToolCall)
                .map_err(D::Error::custom),
            "web_search" => serde_json::from_value(value)
                .map(Self::WebSearch)
                .map_err(D::Error::custom),
            "todo_list" => serde_json::from_value(value)
                .map(Self::TodoList)
                .map_err(D::Error::custom),
            "error" => serde_json::from_value(value)
                .map(Self::Error)
                .map_err(D::Error::custom),
            _ => Ok(Self::Other {
                type_name,
                raw: value,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AgentMessageItem {
    pub text: String,
}

/// Status reported for a command execution item.
///
/// Defaults to `InProgress` on missing values and falls through to `Unknown`
/// on unrecognized values so forward-compat additions do not abort parsing.
#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandExecutionStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    Declined,
    #[serde(other)]
    Unknown,
}

/// A `command_execution` item emitted by `codex exec --json`.
///
/// Only fields observed in codex CLI 0.118+ are modeled. Extra fields like
/// `cwd` / `duration_ms` are defensively kept as `Option` because they have
/// appeared in some shipped versions even though the upstream struct does
/// not list them.
#[derive(Debug, Deserialize)]
pub struct CommandExecutionItem {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub aggregated_output: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub status: CommandExecutionStatus,
    #[serde(default)]
    pub duration_ms: Option<i64>,
}

/// Status reported for a `file_change` item.
#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatchApplyStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct FileChangeItem {
    #[serde(default)]
    pub changes: Vec<FileChangeEntry>,
    #[serde(default)]
    pub status: PatchApplyStatus,
}

#[derive(Debug, Deserialize)]
pub struct FileChangeEntry {
    pub path: String,
    pub kind: String,
    #[serde(default)]
    pub diff: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReasoningItem {
    pub text: String,
}

/// Status reported for an MCP tool call item.
#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpToolCallStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Unknown,
}

/// Result payload from a successful MCP tool invocation.
#[derive(Debug, Deserialize)]
pub struct McpToolCallItemResult {
    #[serde(default)]
    pub content: Vec<serde_json::Value>,
    #[serde(default)]
    pub structured_content: Option<serde_json::Value>,
}

/// Error details reported by a failed MCP tool invocation.
#[derive(Debug, Deserialize)]
pub struct McpToolCallItemError {
    pub message: String,
}

/// A `mcp_tool_call` item emitted by `codex exec --json`.
#[derive(Debug, Deserialize)]
pub struct McpToolCallItem {
    pub server: String,
    pub tool: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub result: Option<McpToolCallItemResult>,
    #[serde(default)]
    pub error: Option<McpToolCallItemError>,
    #[serde(default)]
    pub status: McpToolCallStatus,
}

/// Status reported for a collab tool call item.
#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollabToolCallStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Unknown,
}

/// The supported collab tools reported by codex.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollabTool {
    SpawnAgent,
    SendInput,
    Wait,
    CloseAgent,
    #[serde(other)]
    Unknown,
}

/// Status of a collab agent involved in a collab tool call.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollabAgentStatus {
    PendingInit,
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
    NotFound,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct CollabAgentState {
    pub status: CollabAgentStatus,
    #[serde(default)]
    pub message: Option<String>,
}

/// A `collab_tool_call` item emitted by `codex exec --json`.
#[derive(Debug, Deserialize)]
pub struct CollabToolCallItem {
    pub tool: CollabTool,
    pub sender_thread_id: String,
    #[serde(default)]
    pub receiver_thread_ids: Vec<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub agents_states: HashMap<String, CollabAgentState>,
    #[serde(default)]
    pub status: CollabToolCallStatus,
}

/// A `web_search` item emitted by `codex exec --json`. Upstream's
/// `WebSearchAction` is an enum of shapes specific to the underlying
/// provider; we keep it as a raw JSON value since we do not introspect it.
///
/// Note: upstream `WebSearchItem` has an inner `id` field that collides on
/// the wire with `ThreadItem.id`. We drop it here because the outer
/// `ThreadItem.id` is the only id we care about when publishing.
#[derive(Debug, Deserialize)]
pub struct WebSearchItem {
    pub query: String,
    #[serde(default)]
    pub action: serde_json::Value,
}

/// An `error` item — a non-fatal error surfaced alongside other items in the
/// same turn. The turn is NOT failed by these; they are reported as text so
/// the user can see them without marking the turn red.
#[derive(Debug, Deserialize)]
pub struct ErrorItem {
    pub message: String,
}

/// A `todo_list` item emitted by `codex exec --json` as a running plan
/// tracker. The same `item_id` is re-emitted on every progress update
/// (`item.started` → `item.updated` → `item.completed`).
#[derive(Debug, Deserialize)]
pub struct TodoListItem {
    #[serde(default)]
    pub items: Vec<TodoListEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TodoListEntry {
    pub text: String,
    pub completed: bool,
}

/// Hardcoded final fallback model for Codex sessions.
///
/// Used when neither the request, the repo `agent_config.model`, nor the
/// `DEFAULT_CODEX_MODEL` env var supplies a model. Picked deliberately over
/// the CLI's own auto-default: at the time of writing, the CLI's default
/// (`gpt-5.2-codex`) is the only model in the 5.x family rejected by the
/// ChatGPT-account Codex backend with HTTP 400 "model not supported",
/// which silently breaks every ChatGPT-auth task.
pub const DEFAULT_CODEX_MODEL: &str = "gpt-5.5";

/// Resolve the Codex model with the same priority as Claude:
/// request > repo `agent_config.model` > `DEFAULT_CODEX_MODEL` env > hardcoded default.
pub fn resolve_codex_model(config: &Config, request: &AgentRunRequest) -> String {
    request
        .model
        .clone()
        .or_else(|| config.agent_config.model.clone())
        .or_else(|| config.default_codex_model.clone())
        .unwrap_or_else(|| DEFAULT_CODEX_MODEL.to_string())
}

pub async fn start(
    prompt: &str,
    config: &Config,
    request: &AgentRunRequest,
    env: Option<HashMap<String, String>>,
    failure_detector: CredentialFailureDetector,
) -> Result<CodexCliSession, std::io::Error> {
    let mut command = Command::new(&config.codex_cli_path);
    command.kill_on_drop(true);
    // Detach stdin so codex doesn't block reading an inherited empty stdin
    // (observed with codex-cli >= 0.118: a stdin-append path waits forever
    // when the parent has no controlling tty — sandbox runs as PID 1).
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.current_dir(&config.workspace_dir);
    command.arg("exec");
    command.arg("--json");
    // Codex's built-in sandbox relies on bubblewrap namespaces, which are not
    // available in our Docker task containers. The task container itself is the
    // external sandbox boundary, so run Codex without its internal sandbox.
    command.arg("--dangerously-bypass-approvals-and-sandbox");
    command.arg("--skip-git-repo-check");
    command.arg("--cd");
    command.arg(&config.workspace_dir);

    let model = resolve_codex_model(config, request);
    info!(model = %model, "Codex model resolved");
    command.arg("--model");
    command.arg(&model);

    let developer_instructions = build_prompt_append(config, request);
    command.arg("-c");
    command.arg(format!("developer_instructions={developer_instructions:?}"));

    match &request.resume {
        Some(ResumeSession::WithId(session_id)) => {
            command.arg("resume");
            command.arg(session_id);
            command.arg(prompt);
        }
        Some(ResumeSession::Last) => {
            command.arg("resume");
            command.arg("--last");
            command.arg(prompt);
        }
        None => {
            command.arg(prompt);
        }
    }
    command.env("CODEX_HOME", &config.codex_home_dir);
    command.env("HOME", &config.home_dir);

    if let Some(vars) = env {
        command.envs(vars);
    }

    info!(
        cli = %config.codex_cli_path,
        cwd = %config.workspace_dir,
        home = %config.home_dir,
        codex_home = %config.codex_home_dir,
        "Spawning Codex CLI"
    );
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let cancel_token = CancellationToken::new();
    let (tx, rx) = mpsc::unbounded_channel();
    let session_cancel = cancel_token.clone();

    if let Some(stderr) = stderr {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Scan for credential-failure markers (e.g. refresh_token_reused)
                // before logging — sets the per-credential atomic flag the
                // agent loop polls at turn-failure boundaries.
                failure_detector.scan(&line);
                info!(line, "Codex stderr");
            }
            info!("Codex stderr stream ended");
        });
    }

    tokio::spawn(async move {
        let Some(stdout) = stdout else {
            let _ = tx.send(Err("Codex process did not expose stdout".to_string()));
            let _ = child.kill().await;
            return;
        };

        info!("Codex stdout reader started");
        let mut lines = BufReader::new(stdout).lines();
        let mut saw_terminal_event = false;
        let mut event_count: u32 = 0;

        loop {
            tokio::select! {
                () = session_cancel.cancelled() => {
                    let _ = child.kill().await;
                    break;
                }
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            event_count += 1;
                            info!(event_count, line_len = line.len(), "Codex stdout line received");
                            match serde_json::from_str::<CodexExecEvent>(&line) {
                                Ok(event) => {
                                    if matches!(event, CodexExecEvent::TurnCompleted(_) | CodexExecEvent::TurnFailed(_) | CodexExecEvent::Error(_)) {
                                        saw_terminal_event = true;
                                    }
                                    if tx.send(Ok(event)).is_err() {
                                        info!("Codex event channel closed by receiver");
                                        let _ = child.kill().await;
                                        break;
                                    }
                                }
                                Err(error) => {
                                    info!(error = %error, line = %line, "Codex parse failed");
                                    let _ = tx.send(Err(format!("Failed to parse Codex JSONL event: {error}")));
                                    break;
                                }
                            }
                        }
                        Ok(None) => {
                            info!(event_count, "Codex stdout EOF");
                            break;
                        }
                        Err(error) => {
                            info!(error = %error, "Codex stdout read error");
                            let _ = tx.send(Err(format!("Failed to read Codex stdout: {error}")));
                            break;
                        }
                    }
                }
            }
        }

        info!(
            event_count,
            saw_terminal_event,
            cancelled = session_cancel.is_cancelled(),
            "Codex stdout loop exited, waiting for child"
        );
        match child.wait().await {
            Ok(status)
                if !status.success() && !session_cancel.is_cancelled() && !saw_terminal_event =>
            {
                info!(%status, "Codex child exited with error, no terminal event");
                let _ = tx.send(Err(format!("Codex process exited with status {status}")));
            }
            Ok(status)
                if status.success() && !session_cancel.is_cancelled() && !saw_terminal_event =>
            {
                info!(%status, "Codex child exited success, no terminal event");
                let _ = tx.send(Err(
                    "Codex process exited without a terminal event".to_string()
                ));
            }
            Ok(status) => {
                info!(%status, "Codex child exited cleanly");
            }
            Err(error) if !session_cancel.is_cancelled() => {
                info!(error = %error, "Codex child wait failed");
                let _ = tx.send(Err(format!("Failed to wait for Codex process: {error}")));
            }
            Err(error) => {
                warn!(error = %error, "Failed to wait for cancelled Codex process");
            }
        }
    });

    Ok(CodexCliSession {
        messages: rx,
        cancel_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> CodexExecEvent {
        serde_json::from_str::<CodexExecEvent>(line).unwrap_or_else(|e| panic!("parse: {e}"))
    }

    #[test]
    fn parses_thread_started() {
        let ev = parse(r#"{"type":"thread.started","thread_id":"abc-123"}"#);
        match ev {
            CodexExecEvent::ThreadStarted(e) => assert_eq!(e.thread_id, "abc-123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_agent_message_item() {
        let ev = parse(
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"hi"}}"#,
        );
        match ev {
            CodexExecEvent::ItemCompleted(ItemEvent { item }) => {
                assert_eq!(item.id, "item_0");
                match item.details {
                    ThreadItemDetails::AgentMessage(m) => assert_eq!(m.text, "hi"),
                    _ => panic!("wrong details"),
                }
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_command_execution_completed() {
        let line = r#"{"type":"item.completed","item":{"id":"item_5","type":"command_execution","command":"/bin/sh -c 'python3 hello.py'","aggregated_output":"hi\n","exit_code":0,"status":"completed"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::CommandExecution(c) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(c.command, "/bin/sh -c 'python3 hello.py'");
        assert_eq!(c.aggregated_output.as_deref(), Some("hi\n"));
        assert_eq!(c.exit_code, Some(0));
        assert_eq!(c.status, CommandExecutionStatus::Completed);
    }

    #[test]
    fn parses_command_execution_failed_with_nonzero_exit() {
        let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"/bin/sh -c false","aggregated_output":"","exit_code":1,"status":"failed"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::CommandExecution(c) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(c.exit_code, Some(1));
        assert_eq!(c.status, CommandExecutionStatus::Failed);
    }

    #[test]
    fn parses_command_execution_in_progress_null_exit() {
        let line = r#"{"type":"item.started","item":{"id":"item_5","type":"command_execution","command":"ls","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemStarted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::CommandExecution(c) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(c.exit_code, None);
        assert_eq!(c.status, CommandExecutionStatus::InProgress);
    }

    #[test]
    fn parses_file_change_multi() {
        let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"file_change","changes":[{"path":"/tmp/hello.py","kind":"add"},{"path":"/tmp/other.py","kind":"update"}],"status":"completed"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::FileChange(f) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(f.changes.len(), 2);
        assert_eq!(f.changes[0].path, "/tmp/hello.py");
        assert_eq!(f.changes[0].kind, "add");
        assert_eq!(f.changes[1].kind, "update");
        assert_eq!(f.status, PatchApplyStatus::Completed);
    }

    #[test]
    fn parses_reasoning() {
        let line = r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"thinking step by step"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::Reasoning(r) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(r.text, "thinking step by step");
    }

    #[test]
    fn parses_todo_list_started_all_incomplete() {
        let line = r#"{"type":"item.started","item":{"id":"item_1","type":"todo_list","items":[{"text":"step one","completed":false},{"text":"step two","completed":false},{"text":"step three","completed":false}]}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemStarted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::TodoList(todo) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(todo.items.len(), 3);
        assert!(todo.items.iter().all(|i| !i.completed));
        assert_eq!(todo.items[0].text, "step one");
    }

    #[test]
    fn parses_todo_list_updated_mixed_status() {
        let line = r#"{"type":"item.updated","item":{"id":"item_1","type":"todo_list","items":[{"text":"step one","completed":true},{"text":"step two","completed":true},{"text":"step three","completed":false}]}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemUpdated(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::TodoList(todo) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(todo.items.len(), 3);
        assert!(todo.items[0].completed);
        assert!(todo.items[1].completed);
        assert!(!todo.items[2].completed);
    }

    #[test]
    fn unknown_item_kind_falls_through_to_other_with_raw_payload() {
        let line = r#"{"type":"item.completed","item":{"id":"item_9","type":"brand_new_thing","foo":"bar","count":3}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        match item.details {
            ThreadItemDetails::Other { type_name, raw } => {
                assert_eq!(type_name, "brand_new_thing");
                assert_eq!(raw["foo"], "bar");
                assert_eq!(raw["count"], 3);
            }
            _ => panic!("expected Other"),
        }
    }

    #[test]
    fn parses_mcp_tool_call_completed() {
        let line = r#"{"type":"item.completed","item":{"id":"item_4","type":"mcp_tool_call","server":"atlassian","tool":"jira_search","arguments":{"q":"foo"},"result":{"content":[{"type":"text","text":"match"}],"structured_content":null},"error":null,"status":"completed"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::McpToolCall(mcp) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(mcp.server, "atlassian");
        assert_eq!(mcp.tool, "jira_search");
        assert_eq!(mcp.arguments["q"], "foo");
        assert_eq!(mcp.status, McpToolCallStatus::Completed);
        let result = mcp.result.expect("result present");
        assert_eq!(result.content.len(), 1);
    }

    #[test]
    fn parses_mcp_tool_call_failed() {
        let line = r#"{"type":"item.completed","item":{"id":"item_5","type":"mcp_tool_call","server":"db","tool":"query","arguments":{},"result":null,"error":{"message":"connection refused"},"status":"failed"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::McpToolCall(mcp) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(mcp.status, McpToolCallStatus::Failed);
        let error = mcp.error.expect("error present");
        assert_eq!(error.message, "connection refused");
    }

    #[test]
    fn parses_collab_tool_call() {
        let line = r#"{"type":"item.completed","item":{"id":"item_6","type":"collab_tool_call","tool":"spawn_agent","sender_thread_id":"main","receiver_thread_ids":["sub_a","sub_b"],"prompt":"help","agents_states":{"sub_a":{"status":"running","message":null}},"status":"in_progress"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::CollabToolCall(collab) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(collab.tool, CollabTool::SpawnAgent);
        assert_eq!(collab.sender_thread_id, "main");
        assert_eq!(collab.receiver_thread_ids, vec!["sub_a", "sub_b"]);
        assert_eq!(collab.prompt.as_deref(), Some("help"));
        assert_eq!(collab.status, CollabToolCallStatus::InProgress);
        let state = collab.agents_states.get("sub_a").expect("agent state");
        assert_eq!(state.status, CollabAgentStatus::Running);
    }

    #[test]
    fn parses_web_search_item() {
        let line = r#"{"type":"item.completed","item":{"id":"item_7","type":"web_search","query":"rust release","action":{"type":"search"}}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::WebSearch(ws) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(item.id, "item_7");
        assert_eq!(ws.query, "rust release");
        assert_eq!(ws.action["type"], "search");
    }

    #[test]
    fn parses_error_item() {
        let line = r#"{"type":"item.completed","item":{"id":"item_8","type":"error","message":"tool returned warning"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::Error(err) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(err.message, "tool returned warning");
    }

    #[test]
    fn unknown_command_status_falls_back_to_unknown() {
        let line = r#"{"type":"item.completed","item":{"id":"item_10","type":"command_execution","command":"ls","aggregated_output":"","exit_code":0,"status":"quantum_entangled"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        let ThreadItemDetails::CommandExecution(c) = item.details else {
            panic!("wrong details");
        };
        assert_eq!(c.status, CommandExecutionStatus::Unknown);
    }

    fn model_test_config(
        repo_model: Option<&str>,
        default_codex_model: Option<&str>,
    ) -> Config {
        use crate::config::{AgentConfig, ProxyConfig};
        Config {
            port: 0,
            grpc_port: 0,
            task_id: "task-test".to_string(),
            workspace_dir: "/tmp/workspace".to_string(),
            repo_dir: "/tmp/workspace".to_string(),
            data_dir: "/tmp/data".to_string(),
            upload_dir: "/tmp/uploads".to_string(),
            log_level: "info".to_string(),
            backend_grpc_url: "http://localhost:50051".to_string(),
            backend_api_url: None,
            user_auth_token: None,
            bootstrap_token: None,
            agent_config: AgentConfig {
                model: repo_model.map(String::from),
                ..AgentConfig::default()
            },
            claude_cli_path: "/usr/bin/claude".to_string(),
            codex_cli_path: "/usr/bin/codex".to_string(),
            proxy: ProxyConfig::default(),
            servers: vec![],
            frontend_url: None,
            use_tmux: false,
            permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
            workspace_sync_enabled: false,
            default_model: None,
            default_codex_model: default_codex_model.map(String::from),
            home_dir: "/tmp/home".to_string(),
            codex_home_dir: "/tmp/home/.codex".to_string(),
            claude_config_dir: "/tmp/home/.claude".to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repo_config_raw: None,
            setup_commands: vec![],
            plugin_ids: vec![],
            plugin_repos: vec![],
        }
    }

    fn model_test_request(req_model: Option<&str>) -> AgentRunRequest {
        AgentRunRequest {
            content: vec![],
            model: req_model.map(String::from),
            system_prompt: None,
            resume: None,
            agent_type: None,
            agent_mode: None,
        }
    }

    #[test]
    fn model_request_wins_over_repo_and_env_and_default() {
        let cfg = model_test_config(Some("repo-model"), Some("env-model"));
        let req = model_test_request(Some("req-model"));
        assert_eq!(resolve_codex_model(&cfg, &req), "req-model");
    }

    #[test]
    fn model_repo_wins_over_env_and_default() {
        let cfg = model_test_config(Some("repo-model"), Some("env-model"));
        let req = model_test_request(None);
        assert_eq!(resolve_codex_model(&cfg, &req), "repo-model");
    }

    #[test]
    fn model_env_wins_over_default() {
        let cfg = model_test_config(None, Some("env-model"));
        let req = model_test_request(None);
        assert_eq!(resolve_codex_model(&cfg, &req), "env-model");
    }

    #[test]
    fn model_falls_back_to_hardcoded_default_when_all_unset() {
        let cfg = model_test_config(None, None);
        let req = model_test_request(None);
        assert_eq!(resolve_codex_model(&cfg, &req), DEFAULT_CODEX_MODEL);
        // Sanity-check the chosen default is not the broken one.
        assert_ne!(DEFAULT_CODEX_MODEL, "gpt-5.2-codex");
    }
}
