use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

use super::agents::AgentDefinition;
use super::mcp::McpServerConfig;
use super::messages::SdkMessage;
use super::permissions::{PermissionMode, PermissionUpdate};

/// Raw message from the CLI's stdout. This is the top-level envelope
/// that wraps SDK messages, control protocol, and keep-alive signals.
#[derive(Debug, Clone)]
pub enum RawStdoutMessage {
    /// A regular SDK message (assistant, user, result, system, etc.).
    Sdk(SdkMessage),
    /// An incoming control request from CLI → SDK.
    IncomingControlRequest {
        request_id: String,
        request: IncomingControlRequestInner,
    },
    /// A response to an outgoing control request.
    ControlResponse(ControlResponseInner),
    /// A cancellation of a pending control request.
    ControlCancelRequest { request_id: String },
    /// Keep-alive signal.
    KeepAlive,
    /// Unknown message type — preserved as raw JSON.
    Unknown(Value),
}

/// Incoming control requests from the CLI → SDK (3 subtypes).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "subtype")]
pub enum IncomingControlRequestInner {
    /// Permission check for a tool use.
    #[serde(rename = "can_use_tool")]
    CanUseTool {
        tool_name: String,
        input: Map<String, Value>,
        tool_use_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        permission_suggestions: Option<Vec<PermissionUpdate>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        blocked_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        decision_reason: Option<String>,
    },
    /// Hook callback from the CLI.
    #[serde(rename = "hook_callback")]
    HookCallback {
        callback_id: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
    },
    /// MCP message forwarded from the CLI.
    #[serde(rename = "mcp_message")]
    McpMessage {
        server_name: String,
        message: Value,
    },
}

/// Outgoing control requests from SDK → CLI (12 subtypes).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "subtype")]
pub enum OutgoingControlRequest {
    /// Initialize the SDK session with hooks, agents, etc.
    #[serde(rename = "initialize")]
    Initialize {
        #[serde(skip_serializing_if = "Option::is_none")]
        hooks: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "sdkMcpServers")]
        sdk_mcp_servers: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "jsonSchema")]
        json_schema: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "systemPrompt")]
        system_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "appendSystemPrompt")]
        append_system_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        agents: Option<HashMap<String, AgentDefinition>>,
    },
    /// Interrupt the current turn.
    #[serde(rename = "interrupt")]
    Interrupt,
    /// Change the permission mode.
    #[serde(rename = "set_permission_mode")]
    SetPermissionMode { mode: PermissionMode },
    /// Change the model.
    #[serde(rename = "set_model")]
    SetModel {
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// Set max thinking tokens.
    #[serde(rename = "set_max_thinking_tokens")]
    SetMaxThinkingTokens {
        #[serde(skip_serializing_if = "Option::is_none")]
        max_thinking_tokens: Option<i64>,
    },
    /// Rewind files to a previous state.
    #[serde(rename = "rewind_files")]
    RewindFiles {
        user_message_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        dry_run: Option<bool>,
    },
    /// Request MCP server status.
    #[serde(rename = "mcp_status")]
    McpStatus,
    /// Set MCP servers.
    #[serde(rename = "mcp_set_servers")]
    McpSetServers {
        servers: HashMap<String, McpServerConfig>,
    },
    /// Reconnect an MCP server.
    #[serde(rename = "mcp_reconnect")]
    McpReconnect {
        #[serde(rename = "serverName")]
        server_name: String,
    },
    /// Toggle an MCP server on/off.
    #[serde(rename = "mcp_toggle")]
    McpToggle {
        #[serde(rename = "serverName")]
        server_name: String,
        enabled: bool,
    },
}

/// Response to a control request (success or error).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "subtype")]
pub enum ControlResponseInner {
    #[serde(rename = "success")]
    Success {
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        response: Option<Value>,
    },
    #[serde(rename = "error")]
    Error {
        request_id: String,
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pending_permission_requests: Option<Vec<Value>>,
    },
}

/// Wrapper for writing control requests to stdin.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ControlRequestEnvelope {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: String,
    pub request: OutgoingControlRequest,
}

/// Wrapper for writing control responses to stdin.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ControlResponseEnvelope {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub response: ControlResponsePayload,
}

/// Payload for a control response written to stdin.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ControlResponsePayload {
    pub subtype: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RawStdoutMessage {
    /// Parse a raw JSON value into a `RawStdoutMessage`.
    pub fn from_value(value: Value) -> Self {
        let msg_type = value.get("type").and_then(|v| v.as_str());

        match msg_type {
            Some("control_request") => {
                let request_id = value
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                match serde_json::from_value::<IncomingControlRequestInner>(
                    value.get("request").cloned().unwrap_or(Value::Null),
                ) {
                    Ok(request) => RawStdoutMessage::IncomingControlRequest {
                        request_id,
                        request,
                    },
                    Err(_) => RawStdoutMessage::Unknown(value),
                }
            }
            Some("control_response") => {
                match serde_json::from_value::<ControlResponseInner>(
                    value.get("response").cloned().unwrap_or(Value::Null),
                ) {
                    Ok(response) => RawStdoutMessage::ControlResponse(response),
                    Err(_) => RawStdoutMessage::Unknown(value),
                }
            }
            Some("control_cancel_request") => {
                let request_id = value
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                RawStdoutMessage::ControlCancelRequest { request_id }
            }
            Some("keep_alive") => RawStdoutMessage::KeepAlive,
            _ => {
                // Try to parse as an SDK message
                match SdkMessage::from_value(value.clone()) {
                    Some(msg) => RawStdoutMessage::Sdk(msg),
                    None => RawStdoutMessage::Unknown(value),
                }
            }
        }
    }
}
