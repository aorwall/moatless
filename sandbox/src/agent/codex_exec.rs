use std::collections::HashMap;
use std::process::Stdio;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

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
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct AgentMessageItem {
    pub text: String,
}

pub async fn start(
    prompt: &str,
    config: &Config,
    request: &AgentRunRequest,
    env: Option<HashMap<String, String>>,
) -> Result<CodexCliSession, std::io::Error> {
    let mut command = Command::new(&config.codex_cli_path);
    command.kill_on_drop(true);
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
                debug!(line, "Codex stderr");
            }
        });
    }

    tokio::spawn(async move {
        let Some(stdout) = stdout else {
            let _ = tx.send(Err("Codex process did not expose stdout".to_string()));
            let _ = child.kill().await;
            return;
        };

        let mut lines = BufReader::new(stdout).lines();
        let mut saw_terminal_event = false;

        loop {
            tokio::select! {
                () = session_cancel.cancelled() => {
                    let _ = child.kill().await;
                    break;
                }
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            match serde_json::from_str::<CodexExecEvent>(&line) {
                                Ok(event) => {
                                    if matches!(event, CodexExecEvent::TurnCompleted(_) | CodexExecEvent::TurnFailed(_) | CodexExecEvent::Error(_)) {
                                        saw_terminal_event = true;
                                    }
                                    if tx.send(Ok(event)).is_err() {
                                        let _ = child.kill().await;
                                        break;
                                    }
                                }
                                Err(error) => {
                                    let _ = tx.send(Err(format!("Failed to parse Codex JSONL event: {error}")));
                                    break;
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            let _ = tx.send(Err(format!("Failed to read Codex stdout: {error}")));
                            break;
                        }
                    }
                }
            }
        }

        match child.wait().await {
            Ok(status)
                if !status.success() && !session_cancel.is_cancelled() && !saw_terminal_event =>
            {
                let _ = tx.send(Err(format!("Codex process exited with status {status}")));
            }
            Ok(status)
                if status.success() && !session_cancel.is_cancelled() && !saw_terminal_event =>
            {
                let _ = tx.send(Err(
                    "Codex process exited without a terminal event".to_string()
                ));
            }
            Ok(_) => {}
            Err(error) if !session_cancel.is_cancelled() => {
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
