use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::error::{Result, SdkError};
use crate::protocol::orchestrator::Orchestrator;
use crate::protocol::request_tracker::RequestTracker;
use crate::transport::SubprocessTransport;
use crate::types::control::{ControlRequestEnvelope, OutgoingControlRequest};
use crate::types::mcp::McpServerConfig;
use crate::types::messages::{SdkMessage, UserMessage};
use crate::types::options::ClaudeOptions;
use crate::types::permissions::PermissionMode;

/// Channel capacity for the message stream.
const CHANNEL_CAPACITY: usize = 256;

/// Handle to a running query. Provides the message stream and control methods.
pub struct QueryHandle {
    /// Stream of SDK messages from the Claude CLI.
    pub messages: mpsc::Receiver<Result<SdkMessage>>,
    orchestrator_handle: JoinHandle<()>,
    stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
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
        let json = serde_json::to_string(message)?;
        let mut stdin = self.stdin.lock().await;
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(json.as_bytes())
            .await
            .map_err(SdkError::SpawnError)?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(SdkError::SpawnError)?;
        stdin.flush().await.map_err(SdkError::SpawnError)?;
        Ok(())
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

        {
            let mut stdin = self.stdin.lock().await;
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(json.as_bytes())
                .await
                .map_err(SdkError::SpawnError)?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(SdkError::SpawnError)?;
            stdin.flush().await.map_err(SdkError::SpawnError)?;
        }

        // Wait for response with timeout
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

/// Start a one-shot query. The prompt is sent as the initial user message,
/// and the returned `QueryHandle` provides the message stream.
pub async fn query(prompt: &str, options: ClaudeOptions) -> Result<QueryHandle> {
    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();
    let permission_handler = options.can_use_tool.clone();

    // 1. Spawn transport
    let transport = SubprocessTransport::spawn(&options).await?;
    let stdin = transport.stdin.clone();

    // 2. Create channels
    let (message_tx, message_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let request_tracker = Arc::new(Mutex::new(RequestTracker::new()));

    // 3. Spawn orchestrator
    let orchestrator_handle = Orchestrator::spawn(
        transport,
        message_tx,
        request_tracker.clone(),
        permission_handler,
        stdin.clone(),
        cancellation_token.clone(),
    );

    // 4. Send initialize control request
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

    {
        let json = serde_json::to_string(&init_envelope)?;
        let mut stdin_guard = stdin.lock().await;
        use tokio::io::AsyncWriteExt;
        stdin_guard
            .write_all(json.as_bytes())
            .await
            .map_err(SdkError::SpawnError)?;
        stdin_guard
            .write_all(b"\n")
            .await
            .map_err(SdkError::SpawnError)?;
        stdin_guard.flush().await.map_err(SdkError::SpawnError)?;
    }

    // Wait for init response
    match tokio::time::timeout(std::time::Duration::from_secs(30), init_rx).await {
        Ok(Ok(Ok(_))) => {
            debug!("Initialize control request succeeded");
        }
        Ok(Ok(Err(error_info))) => {
            return Err(SdkError::ControlError {
                request_id: init_id,
                error: error_info.error,
            });
        }
        Ok(Err(_)) => {
            return Err(SdkError::TransportClosed);
        }
        Err(_) => {
            return Err(SdkError::ControlTimeout {
                request_id: init_id,
                timeout_ms: 30_000,
            });
        }
    }

    // 5. Write user prompt as NDJSON
    let user_msg = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": prompt}]
        }
    });

    {
        let json = serde_json::to_string(&user_msg)?;
        let mut stdin_guard = stdin.lock().await;
        use tokio::io::AsyncWriteExt;
        stdin_guard
            .write_all(json.as_bytes())
            .await
            .map_err(SdkError::SpawnError)?;
        stdin_guard
            .write_all(b"\n")
            .await
            .map_err(SdkError::SpawnError)?;
        stdin_guard.flush().await.map_err(SdkError::SpawnError)?;
    }

    Ok(QueryHandle {
        messages: message_rx,
        orchestrator_handle,
        stdin,
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

    // 1. Spawn transport
    let transport = SubprocessTransport::spawn(&options).await?;
    let stdin = transport.stdin.clone();

    // 2. Create channels
    let (message_tx, message_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let request_tracker = Arc::new(Mutex::new(RequestTracker::new()));

    // 3. Spawn orchestrator
    let orchestrator_handle = Orchestrator::spawn(
        transport,
        message_tx,
        request_tracker.clone(),
        permission_handler,
        stdin.clone(),
        cancellation_token.clone(),
    );

    // 4. Send initialize control request
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

    {
        let json = serde_json::to_string(&init_envelope)?;
        let mut stdin_guard = stdin.lock().await;
        use tokio::io::AsyncWriteExt;
        stdin_guard
            .write_all(json.as_bytes())
            .await
            .map_err(SdkError::SpawnError)?;
        stdin_guard
            .write_all(b"\n")
            .await
            .map_err(SdkError::SpawnError)?;
        stdin_guard.flush().await.map_err(SdkError::SpawnError)?;
    }

    // Wait for init response
    match tokio::time::timeout(std::time::Duration::from_secs(30), init_rx).await {
        Ok(Ok(Ok(_))) => {
            debug!("Initialize control request succeeded");
        }
        Ok(Ok(Err(error_info))) => {
            return Err(SdkError::ControlError {
                request_id: init_id,
                error: error_info.error,
            });
        }
        Ok(Err(_)) => {
            return Err(SdkError::TransportClosed);
        }
        Err(_) => {
            return Err(SdkError::ControlTimeout {
                request_id: init_id,
                timeout_ms: 30_000,
            });
        }
    }

    Ok(QueryHandle {
        messages: message_rx,
        orchestrator_handle,
        stdin,
        request_tracker,
        cancellation_token,
    })
}

/// Build the initialize control request from options.
fn build_initialize_request(options: &ClaudeOptions) -> OutgoingControlRequest {
    let system_prompt = match &options.system_prompt {
        Some(crate::types::options::SystemPrompt::Replace(text)) => Some(text.clone()),
        _ => None,
    };

    let append_system_prompt = match &options.system_prompt {
        Some(crate::types::options::SystemPrompt::Preset { append, .. }) => append.clone(),
        _ => None,
    };

    let json_schema = options.output_format.as_ref().and_then(|f| {
        serde_json::to_value(f).ok()
    });

    OutgoingControlRequest::Initialize {
        hooks: None, // Hooks are handled via the orchestrator, not serialized
        sdk_mcp_servers: None,
        json_schema,
        system_prompt,
        append_system_prompt,
        agents: options.agents.clone(),
    }
}
