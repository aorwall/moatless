use std::collections::HashMap;
use std::process::Stdio;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;

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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadItemDetails {
    AgentMessage(AgentMessageItem),
    CommandExecution(CommandExecutionItem),
    FileChange(FileChangeItem),
    Reasoning(ReasoningItem),
    TodoList(TodoListItem),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct AgentMessageItem {
    pub text: String,
}

/// A `command_execution` item emitted by `codex exec --json`.
///
/// Only fields observed in codex CLI 0.118+ are modeled. Forward-compat:
/// unknown fields are ignored by serde's default.
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
    pub status: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct FileChangeItem {
    #[serde(default)]
    pub changes: Vec<FileChangeEntry>,
    #[serde(default)]
    pub status: Option<String>,
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

pub async fn start(
    prompt: &str,
    config: &Config,
    request: &AgentRunRequest,
    env: Option<HashMap<String, String>>,
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

    if let Some(ref model) = request.model {
        command.arg("--model");
        command.arg(model);
    }

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
        assert_eq!(c.status.as_deref(), Some("completed"));
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
        assert_eq!(c.status.as_deref(), Some("failed"));
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
        assert_eq!(c.status.as_deref(), Some("in_progress"));
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
        assert_eq!(f.status.as_deref(), Some("completed"));
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
    fn unknown_item_kind_falls_through_to_other() {
        let line = r#"{"type":"item.completed","item":{"id":"item_9","type":"mcp_tool_call","name":"foo"}}"#;
        let ev = parse(line);
        let CodexExecEvent::ItemCompleted(ItemEvent { item }) = ev else {
            panic!("wrong variant");
        };
        assert!(matches!(item.details, ThreadItemDetails::Other));
    }
}
