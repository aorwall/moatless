use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::error::{Result, SdkError};
use crate::mcp::SdkMcpServer;
use crate::protocol::orchestrator::Orchestrator;
use crate::protocol::request_tracker::RequestTracker;
use crate::transport::{ChildStdinWriter, StdinWriter, SubprocessTransport};
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
pub async fn query(prompt: &str, mut options: ClaudeOptions) -> Result<QueryHandle> {
    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();
    let permission_handler = options.can_use_tool.clone();
    let hook_handler = options.hook_handler.clone();

    // Extract SDK MCP servers (takes ownership since SdkMcpServer is not Clone)
    let sdk_mcp_servers = options.sdk_mcp_servers.take();
    let sdk_mcp_server_names = extract_server_names(&sdk_mcp_servers);

    // 1. Spawn transport
    let transport = SubprocessTransport::spawn(&options).await?;
    let stdin_writer: Arc<dyn StdinWriter> = Arc::new(ChildStdinWriter {
        inner: transport.stdin.clone(),
    });

    // 2. Create channels
    let (message_tx, message_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let request_tracker = Arc::new(Mutex::new(RequestTracker::new()));

    // 3. Spawn orchestrator
    let orchestrator_handle = Orchestrator::spawn(
        Box::new(transport),
        message_tx,
        request_tracker.clone(),
        permission_handler,
        hook_handler,
        sdk_mcp_servers,
        stdin_writer.clone(),
        cancellation_token.clone(),
    );

    // 4. Send initialize control request
    let init_request = build_initialize_request(&options, sdk_mcp_server_names);
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

    let json = serde_json::to_string(&user_msg)?;
    stdin_writer.write_json_line(&json).await?;

    Ok(QueryHandle {
        messages: message_rx,
        orchestrator_handle,
        stdin_writer,
        request_tracker,
        cancellation_token,
    })
}

/// Start a streaming multi-turn query. Unlike `query()`, stdin is NOT closed
/// after the initial message — additional messages can be sent via
/// `QueryHandle::send_user_message()`.
pub async fn query_streaming(mut options: ClaudeOptions) -> Result<QueryHandle> {
    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();
    let permission_handler = options.can_use_tool.clone();
    let hook_handler = options.hook_handler.clone();

    // Extract SDK MCP servers (takes ownership since SdkMcpServer is not Clone)
    let sdk_mcp_servers = options.sdk_mcp_servers.take();
    let sdk_mcp_server_names = extract_server_names(&sdk_mcp_servers);

    // 1. Spawn transport
    let transport = SubprocessTransport::spawn(&options).await?;
    let stdin_writer: Arc<dyn StdinWriter> = Arc::new(ChildStdinWriter {
        inner: transport.stdin.clone(),
    });

    // 2. Create channels
    let (message_tx, message_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let request_tracker = Arc::new(Mutex::new(RequestTracker::new()));

    // 3. Spawn orchestrator
    let orchestrator_handle = Orchestrator::spawn(
        Box::new(transport),
        message_tx,
        request_tracker.clone(),
        permission_handler,
        hook_handler,
        sdk_mcp_servers,
        stdin_writer.clone(),
        cancellation_token.clone(),
    );

    // 4. Send initialize control request
    let init_request = build_initialize_request(&options, sdk_mcp_server_names);
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
        stdin_writer,
        request_tracker,
        cancellation_token,
    })
}

/// Extract server names from the SDK MCP servers map.
fn extract_server_names(
    servers: &Option<HashMap<String, SdkMcpServer>>,
) -> Option<Vec<String>> {
    servers
        .as_ref()
        .map(|s| s.keys().cloned().collect())
}

/// Build the initialize control request from options.
fn build_initialize_request(
    options: &ClaudeOptions,
    sdk_mcp_server_names: Option<Vec<String>>,
) -> OutgoingControlRequest {
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

    // Serialize hook definitions in the CLI-expected format:
    // Record<HookEvent, Array<{hookCallbackIds: string[], matcher?: string, timeout?: number}>>
    let hooks = options.hooks.as_ref().map(|defs| {
        let mut map: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        for (i, def) in defs.iter().enumerate() {
            let event_name = serde_json::to_value(&def.event)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| format!("{:?}", def.event));
            let callback_id = format!("sdk-hook-{}-{}", event_name, i);
            let mut entry = serde_json::json!({
                "hookCallbackIds": [callback_id],
            });
            if let Some(ref matcher) = def.matcher {
                entry["matcher"] = serde_json::Value::String(matcher.clone());
            }
            if let Some(timeout) = def.timeout {
                entry["timeout"] = serde_json::json!(timeout);
            }
            map.entry(event_name).or_default().push(entry);
        }
        serde_json::to_value(map).unwrap_or(serde_json::Value::Null)
    });

    OutgoingControlRequest::Initialize {
        hooks,
        sdk_mcp_servers: sdk_mcp_server_names,
        json_schema,
        system_prompt,
        append_system_prompt,
        agents: options.agents.clone(),
    }
}
