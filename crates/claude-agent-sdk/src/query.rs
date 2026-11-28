use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::api::control::{ControlRequestEnvelope, OutgoingControlRequest};
use crate::api::mcp::McpServerConfig;
use crate::api::messages::{SdkMessage, UserMessage};
use crate::api::options::ClaudeOptions;
use crate::api::permissions::PermissionMode;
use crate::error::{Result, SdkError};
use crate::protocol::orchestrator::Orchestrator;
use crate::protocol::request_tracker::RequestTracker;
use crate::session_cost::SessionCost;
use crate::transport::{ChildStdinWriter, StdinWriter, SubprocessTransport};

/// Channel capacity for the message stream.
const CHANNEL_CAPACITY: usize = 256;

/// Handle to a running query. Provides the message stream and control methods.
pub struct QueryHandle {
    /// Stream of SDK messages from the Claude CLI.
    pub messages: mpsc::Receiver<Result<SdkMessage>>,
    /// `None` for the subprocess transport, or if tmux could not be queried.
    ///
    /// Lets a caller tell the session apart from work it spawned: this PID and
    /// its ancestors are the session, anything else is the agent's own
    /// background work.
    pub tmux_pane_pid: Option<u32>,
    orchestrator_handle: JoinHandle<()>,
    stdin_writer: Arc<dyn StdinWriter>,
    request_tracker: Arc<Mutex<RequestTracker>>,
    cancellation_token: CancellationToken,
}

impl QueryHandle {
    /// Interrupt the current turn.
    pub async fn interrupt(&self) -> Result<()> {
        self.send_control_request(OutgoingControlRequest::Interrupt)
            .await?;
        Ok(())
    }

    /// Change the permission mode.
    pub async fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        self.send_control_request(OutgoingControlRequest::SetPermissionMode { mode })
            .await?;
        Ok(())
    }

    /// Change the model.
    pub async fn set_model(&self, model: Option<String>) -> Result<()> {
        self.send_control_request(OutgoingControlRequest::SetModel { model })
            .await?;
        Ok(())
    }

    /// Set max thinking tokens.
    pub async fn set_max_thinking_tokens(&self, tokens: Option<i64>) -> Result<()> {
        self.send_control_request(OutgoingControlRequest::SetMaxThinkingTokens {
            max_thinking_tokens: tokens,
        })
        .await?;
        Ok(())
    }

    /// Get the status of all MCP servers.
    pub async fn mcp_status(&self) -> Result<()> {
        self.send_control_request(OutgoingControlRequest::McpStatus)
            .await?;
        Ok(())
    }

    /// Set/replace the MCP server configuration at runtime.
    pub async fn set_mcp_servers(
        &self,
        servers: std::collections::HashMap<String, McpServerConfig>,
    ) -> Result<()> {
        self.send_control_request(OutgoingControlRequest::McpSetServers { servers })
            .await?;
        Ok(())
    }

    /// Reconnect a specific MCP server by name.
    pub async fn reconnect_mcp_server(&self, server_name: String) -> Result<()> {
        self.send_control_request(OutgoingControlRequest::McpReconnect { server_name })
            .await?;
        Ok(())
    }

    /// Toggle an MCP server on or off.
    pub async fn toggle_mcp_server(&self, server_name: String, enabled: bool) -> Result<()> {
        self.send_control_request(OutgoingControlRequest::McpToggle {
            server_name,
            enabled,
        })
        .await?;
        Ok(())
    }

    /// Send a user message to the CLI.
    pub async fn send_user_message(&self, message: &UserMessage) -> Result<()> {
        // The CLI expects `type: "user"` at the top level, but UserMessage
        // doesn't include it. Merge it into the serialized JSON.
        let mut value = serde_json::to_value(message)?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "type".to_string(),
                serde_json::Value::String("user".to_string()),
            );
            // The CLI expects `message.content` as a plain string, not an array
            // of content blocks. Using the array format causes session resume to
            // fail (the CLI can't replay array-format user messages).
            flatten_content_to_string(obj);
        }
        let json = serde_json::to_string(&value)?;
        self.stdin_writer.write_json_line(&json).await
    }

    /// Close the query and terminate the subprocess.
    pub fn close(&self) {
        self.cancellation_token.cancel();
    }

    /// Check if the orchestrator is still running.
    pub fn is_running(&self) -> bool {
        !self.orchestrator_handle.is_finished()
    }

    /// Send a control request and wait for the response.
    async fn send_control_request(&self, request: OutgoingControlRequest) -> Result<()> {
        let (request_id, rx) = {
            let mut tracker = self.request_tracker.lock().await;
            tracker.register()
        };
        // Guard dropped here — safe to await

        let envelope = ControlRequestEnvelope {
            msg_type: "control_request".to_string(),
            request_id: request_id.clone(),
            request,
        };

        let json = serde_json::to_string(&envelope)?;
        self.stdin_writer.write_json_line(&json).await?;

        let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx).await;

        match result {
            Ok(Ok(Ok(_response))) => Ok(()),
            Ok(Ok(Err(error_info))) => Err(SdkError::ControlError {
                request_id,
                error: error_info.error,
            }),
            Ok(Err(_)) => Err(SdkError::TransportClosed),
            Err(_) => Err(SdkError::ControlTimeout {
                request_id,
                timeout_ms: 30_000,
            }),
        }
    }
}

impl QueryHandle {
    /// Construct a `QueryHandle` from raw parts, for the tmux backend which
    /// skips the subprocess orchestrator. `background_task` holds whatever
    /// represents the session: the orchestrator for subprocess, the JSONL
    /// tailer for tmux.
    pub(crate) fn from_parts(
        messages: mpsc::Receiver<Result<SdkMessage>>,
        background_task: JoinHandle<()>,
        stdin_writer: Arc<dyn StdinWriter>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            messages,
            tmux_pane_pid: None,
            orchestrator_handle: background_task,
            stdin_writer,
            request_tracker: Arc::new(Mutex::new(RequestTracker::new())),
            cancellation_token,
        }
    }

    /// Record the tmux pane PID captured at session start.
    pub(crate) fn with_tmux_pane_pid(mut self, pane_pid: Option<u32>) -> Self {
        self.tmux_pane_pid = pane_pid;
        self
    }
}

impl Drop for QueryHandle {
    fn drop(&mut self) {
        // Cancel the token to ensure the orchestrator exits and the subprocess
        // is cleaned up. This is idempotent — safe to call after close() or
        // if the orchestrator already finished.
        self.cancellation_token.cancel();
    }
}

/// Start a one-shot query. The prompt is sent as the initial user message, and
/// the returned `QueryHandle` provides the message stream.
pub async fn query(prompt: &str, options: ClaudeOptions) -> Result<QueryHandle> {
    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();
    let permission_handler = options.can_use_tool.clone();

    let session_cost = SessionCost::restored(&options).await;
    let transport = SubprocessTransport::spawn(&options)?;
    let stdin_writer: Arc<dyn StdinWriter> = Arc::new(ChildStdinWriter {
        inner: transport.stdin.clone(),
    });
    // Capture stderr buffer before transport is moved into the orchestrator
    let stderr_buf = transport.stderr_buffer();

    let (message_tx, message_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let request_tracker = Arc::new(Mutex::new(RequestTracker::new()));

    let orchestrator_handle = Orchestrator::spawn(
        Box::new(transport),
        message_tx,
        request_tracker.clone(),
        permission_handler,
        stdin_writer.clone(),
        cancellation_token.clone(),
        session_cost,
    );

    let init_request = build_initialize_request(&options);
    let (init_id, init_rx) = {
        let mut tracker = request_tracker.lock().await;
        tracker.register()
    };

    let init_envelope = ControlRequestEnvelope {
        msg_type: "control_request".to_string(),
        request_id: init_id.clone(),
        request: init_request,
    };

    let json = serde_json::to_string(&init_envelope)?;
    stdin_writer.write_json_line(&json).await?;

    let init_timeout = options
        .init_timeout
        .unwrap_or(std::time::Duration::from_mins(1));

    info!(
        request_id = %init_id,
        init_timeout_ms = u64::try_from(init_timeout.as_millis()).unwrap_or(u64::MAX),
        "Waiting for initialize response from CLI"
    );

    match tokio::time::timeout(init_timeout, init_rx).await {
        Ok(Ok(Ok(_))) => {
            debug!("Initialize control request succeeded");
        }
        Ok(Ok(Err(error_info))) => {
            warn!(
                request_id = %init_id,
                error = %error_info.error,
                "CLI rejected initialize request"
            );
            cancellation_token.cancel();
            return Err(SdkError::ControlError {
                request_id: init_id,
                error: error_info.error,
            });
        }
        Ok(Err(_)) => {
            warn!(
                request_id = %init_id,
                "Transport closed before init response — CLI may have crashed during startup"
            );
            cancellation_token.cancel();
            return Err(SdkError::TransportClosed);
        }
        Err(_) => {
            let stderr_snapshot = stderr_buf.lock().await;
            let stderr_tail: Vec<&str> = stderr_snapshot
                .iter()
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(String::as_str)
                .collect();
            warn!(
                request_id = %init_id,
                timeout_ms = u64::try_from(init_timeout.as_millis()).unwrap_or(u64::MAX),
                stderr_line_count = stderr_snapshot.len(),
                stderr_tail = ?stderr_tail,
                "CLI init timed out — no initialize response received"
            );
            drop(stderr_snapshot);
            cancellation_token.cancel();
            return Err(SdkError::ControlTimeout {
                request_id: init_id,
                timeout_ms: u64::try_from(init_timeout.as_millis()).unwrap_or(u64::MAX),
            });
        }
    }

    // 5. Write user prompt as NDJSON
    // NOTE: The CLI expects `content` as a plain string for user messages,
    // NOT as an array of content blocks. Using the array format causes
    // session resume to fail (the CLI can't replay array-format messages).
    let user_msg = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": prompt
        }
    });

    let json = serde_json::to_string(&user_msg)?;
    stdin_writer.write_json_line(&json).await?;

    Ok(QueryHandle {
        messages: message_rx,
        tmux_pane_pid: None,
        orchestrator_handle,
        stdin_writer,
        request_tracker,
        cancellation_token,
    })
}

/// Start a streaming multi-turn query. Unlike `query()`, stdin is NOT closed
/// after the initial message — additional messages can be sent via
/// `QueryHandle::send_user_message()`.
pub async fn query_streaming(options: ClaudeOptions) -> Result<QueryHandle> {
    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();
    let permission_handler = options.can_use_tool.clone();

    let session_cost = SessionCost::restored(&options).await;
    let transport = SubprocessTransport::spawn(&options)?;
    let stdin_writer: Arc<dyn StdinWriter> = Arc::new(ChildStdinWriter {
        inner: transport.stdin.clone(),
    });
    // Capture stderr buffer before transport is moved into the orchestrator
    let stderr_buf = transport.stderr_buffer();

    let (message_tx, message_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let request_tracker = Arc::new(Mutex::new(RequestTracker::new()));

    let orchestrator_handle = Orchestrator::spawn(
        Box::new(transport),
        message_tx,
        request_tracker.clone(),
        permission_handler,
        stdin_writer.clone(),
        cancellation_token.clone(),
        session_cost,
    );

    let init_request = build_initialize_request(&options);
    let (init_id, init_rx) = {
        let mut tracker = request_tracker.lock().await;
        tracker.register()
    };

    let init_envelope = ControlRequestEnvelope {
        msg_type: "control_request".to_string(),
        request_id: init_id.clone(),
        request: init_request,
    };

    let json = serde_json::to_string(&init_envelope)?;
    stdin_writer.write_json_line(&json).await?;

    let init_timeout = options
        .init_timeout
        .unwrap_or(std::time::Duration::from_mins(1));

    info!(
        request_id = %init_id,
        init_timeout_ms = u64::try_from(init_timeout.as_millis()).unwrap_or(u64::MAX),
        "Waiting for initialize response from CLI"
    );

    match tokio::time::timeout(init_timeout, init_rx).await {
        Ok(Ok(Ok(_))) => {
            debug!("Initialize control request succeeded");
        }
        Ok(Ok(Err(error_info))) => {
            warn!(
                request_id = %init_id,
                error = %error_info.error,
                "CLI rejected initialize request"
            );
            cancellation_token.cancel();
            return Err(SdkError::ControlError {
                request_id: init_id,
                error: error_info.error,
            });
        }
        Ok(Err(_)) => {
            warn!(
                request_id = %init_id,
                "Transport closed before init response — CLI may have crashed during startup"
            );
            cancellation_token.cancel();
            return Err(SdkError::TransportClosed);
        }
        Err(_) => {
            let stderr_snapshot = stderr_buf.lock().await;
            let stderr_tail: Vec<&str> = stderr_snapshot
                .iter()
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(String::as_str)
                .collect();
            warn!(
                request_id = %init_id,
                timeout_ms = u64::try_from(init_timeout.as_millis()).unwrap_or(u64::MAX),
                stderr_line_count = stderr_snapshot.len(),
                stderr_tail = ?stderr_tail,
                "CLI init timed out — no initialize response received"
            );
            drop(stderr_snapshot);
            cancellation_token.cancel();
            return Err(SdkError::ControlTimeout {
                request_id: init_id,
                timeout_ms: u64::try_from(init_timeout.as_millis()).unwrap_or(u64::MAX),
            });
        }
    }

    Ok(QueryHandle {
        messages: message_rx,
        tmux_pane_pid: None,
        orchestrator_handle,
        stdin_writer,
        request_tracker,
        cancellation_token,
    })
}

/// Convert `message.content` from an array of content blocks to a plain string.
///
/// The CLI expects user message content as a string (`"content": "hello"`), not
/// an array of blocks; text blocks are joined into one string.
fn flatten_content_to_string(obj: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(msg) = obj.get_mut("message").and_then(|m| m.as_object_mut()) {
        if let Some(content) = msg.get("content") {
            if let Some(arr) = content.as_array() {
                let text: String = arr
                    .iter()
                    .filter_map(|block| {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            block.get("text").and_then(|t| t.as_str()).map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                msg.insert("content".to_string(), serde_json::Value::String(text));
            }
        }
    }
}

/// Build the initialize control request from options.
fn build_initialize_request(options: &ClaudeOptions) -> OutgoingControlRequest {
    let system_prompt = match &options.system_prompt {
        Some(crate::api::options::SystemPrompt::Replace(text)) => Some(text.clone()),
        _ => None,
    };

    let append_system_prompt = match &options.system_prompt {
        Some(crate::api::options::SystemPrompt::Preset { append, .. }) => append.clone(),
        _ => None,
    };

    // The bare schema, matching what `--json-schema` carries.
    let json_schema = options
        .output_format
        .as_ref()
        .map(|crate::api::options::OutputFormat::JsonSchema { schema }| schema.clone());

    OutgoingControlRequest::Initialize {
        json_schema,
        system_prompt,
        append_system_prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_content_array_to_string() {
        let mut obj = serde_json::json!({
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "hello world"}]
            }
        });
        flatten_content_to_string(obj.as_object_mut().unwrap());
        assert_eq!(obj["message"]["content"], "hello world");
    }

    #[test]
    fn flatten_content_already_string() {
        let mut obj = serde_json::json!({
            "message": {
                "role": "user",
                "content": "already a string"
            }
        });
        flatten_content_to_string(obj.as_object_mut().unwrap());
        assert_eq!(obj["message"]["content"], "already a string");
    }

    #[test]
    fn flatten_content_multiple_text_blocks() {
        let mut obj = serde_json::json!({
            "message": {
                "role": "user",
                "content": [
                    {"type": "text", "text": "first line"},
                    {"type": "text", "text": "second line"}
                ]
            }
        });
        flatten_content_to_string(obj.as_object_mut().unwrap());
        assert_eq!(obj["message"]["content"], "first line\nsecond line");
    }
}
