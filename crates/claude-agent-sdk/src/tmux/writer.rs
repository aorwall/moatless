//! `StdinWriter` implementation that delivers user text to the TUI via the
//! tmux paste buffer.
//!
//! For each user message we:
//! 1. `tmux load-buffer -b <name> -` (text streamed over stdin).
//! 2. `tmux paste-buffer -t <session>:0 -b <name> -p -d` (bracket-paste so
//!    embedded newlines aren't treated as submit; `-d` deletes the buffer
//!    after).
//! 3. `tmux send-keys -t <session>:0 Enter` to submit the prompt.
//!
//! Control envelopes from `QueryHandle` (interrupt, set_permission_mode, etc.)
//! are not supported by the TUI; we log and drop them.

use async_trait::async_trait;
use serde_json::Value;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{debug, warn};

use crate::error::{Result, SdkError};
use crate::transport::StdinWriter;

pub(crate) struct TmuxPasteWriter {
    pub(crate) session_name: String,
}

impl TmuxPasteWriter {
    /// Send a single named key (tmux key syntax, e.g. `Enter`, `Down`) to the
    /// pane. Used to drive startup dialogs (trust / bypass-permissions) and to
    /// submit prompts.
    pub(crate) async fn send_key(&self, key: &str) -> Result<()> {
        let status = Command::new("tmux")
            .args(["send-keys", "-t", self.session_name.as_str(), key])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(SdkError::SpawnError)?;
        if !status.success() {
            return Err(SdkError::Internal(format!(
                "tmux send-keys {key} exited with status {status:?}"
            )));
        }
        Ok(())
    }

    /// Send a bare Enter keypress to the tmux pane. Used to dismiss the
    /// workspace-trust dialog and to submit a previously pasted prompt.
    pub(crate) async fn send_enter(&self) -> Result<()> {
        self.send_key("Enter").await
    }

    /// Paste `text` into the tmux session and press Enter.
    pub(crate) async fn send_text(&self, text: &str) -> Result<()> {
        let buffer_name = format!("sdk-{}", uuid::Uuid::new_v4().simple());

        // 1. Load text into a named tmux buffer via stdin.
        let mut load = Command::new("tmux")
            .args(["load-buffer", "-b", &buffer_name, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    SdkError::TmuxNotFound
                } else {
                    SdkError::SpawnError(e)
                }
            })?;
        if let Some(mut stdin) = load.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(SdkError::SpawnError)?;
            stdin.shutdown().await.map_err(SdkError::SpawnError)?;
        }
        let status = load.wait().await.map_err(SdkError::SpawnError)?;
        if !status.success() {
            return Err(SdkError::Internal(format!(
                "tmux load-buffer exited with status {status:?}"
            )));
        }

        // Target the session by name; tmux resolves to the active pane in
        // the active window. Do NOT use `:0` — `base-index` may be non-zero.
        let target = self.session_name.as_str();

        // 2. Paste with bracket-paste mode (-p) and delete the buffer after (-d).
        let status = Command::new("tmux")
            .args([
                "paste-buffer",
                "-t",
                target,
                "-b",
                &buffer_name,
                "-p",
                "-d",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(SdkError::SpawnError)?;
        if !status.success() {
            return Err(SdkError::Internal(format!(
                "tmux paste-buffer exited with status {status:?}"
            )));
        }

        // Small gap before submit. Ink applies a bracketed-paste / burst-input
        // heuristic: if Enter arrives in the same burst as the prompt body it
        // lands in the input buffer instead of triggering submit. 120ms is
        // enough for Ink to separate the events (claude-p uses the same gap).
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;

        // 3. Send Enter to submit.
        let status = Command::new("tmux")
            .args(["send-keys", "-t", target, "Enter"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(SdkError::SpawnError)?;
        if !status.success() {
            return Err(SdkError::Internal(format!(
                "tmux send-keys Enter exited with status {status:?}"
            )));
        }

        debug!(target = %target, bytes = text.len(), "Pasted user message into tmux");
        Ok(())
    }
}

#[async_trait]
impl StdinWriter for TmuxPasteWriter {
    async fn write_json_line(&self, json: &str) -> Result<()> {
        // The only JSON the SDK writes to the writer in normal flow is a
        // serialized user message (via QueryHandle::send_user_message). Other
        // control envelopes have no equivalent in TUI mode.
        let value: Value = serde_json::from_str(json).map_err(SdkError::JsonError)?;
        let msg_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        if msg_type != "user" {
            warn!(
                msg_type,
                "TmuxPasteWriter dropped non-user message — \
                 control protocol is not supported in tmux mode"
            );
            return Ok(());
        }

        let text = extract_user_text(&value).unwrap_or_default();
        self.send_text(&text).await
    }
}

/// Extract the user-visible text from a serialized `UserMessage` JSON.
/// Handles both string and array-of-blocks `content` shapes.
fn extract_user_text(value: &Value) -> Option<String> {
    let content = value.get("message").and_then(|m| m.get("content"))?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = content.as_array() {
        let joined: String = arr
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    b.get("text").and_then(Value::as_str).map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Some(joined);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_text_from_string_content() {
        let v = json!({
            "type": "user",
            "message": { "role": "user", "content": "hello world" }
        });
        assert_eq!(extract_user_text(&v).as_deref(), Some("hello world"));
    }

    #[test]
    fn extract_text_from_array_content() {
        let v = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "text", "text": "line one"},
                    {"type": "text", "text": "line two"}
                ]
            }
        });
        assert_eq!(
            extract_user_text(&v).as_deref(),
            Some("line one\nline two")
        );
    }
}
