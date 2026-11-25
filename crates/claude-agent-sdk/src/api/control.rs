use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

use super::mcp::McpServerConfig;
use super::messages::SdkMessage;
use super::permissions::{PermissionMode, PermissionUpdate};

/// Raw message from the CLI's stdout. This is the top-level envelope that wraps
/// SDK messages, control protocol, and keep-alive signals.
#[derive(Debug, Clone)]
pub enum RawStdoutMessage {
    /// A regular SDK message (assistant, user, result, system, etc.).
    Sdk(Box<SdkMessage>),
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
    McpMessage { server_name: String, message: Value },
}

/// Outgoing control requests from SDK → CLI (12 subtypes).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "subtype")]
pub enum OutgoingControlRequest {
    /// Initialize the SDK session.
    #[serde(rename = "initialize")]
    Initialize {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "jsonSchema")]
        json_schema: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "systemPrompt")]
        system_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "appendSystemPrompt")]
        append_system_prompt: Option<String>,
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
    #[allow(clippy::should_implement_trait)] // Not a FromStr-like conversion
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
            _ => match SdkMessage::from_value(value.clone()) {
                Some(msg) => RawStdoutMessage::Sdk(Box::new(msg)),
                None => RawStdoutMessage::Unknown(value),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- RawStdoutMessage::from_value parsing tests --

    #[test]
    fn parse_control_request_can_use_tool() {
        let json = serde_json::json!({
            "type": "control_request",
            "request_id": "req-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "input": {"command": "ls"},
                "tool_use_id": "tu-1"
            }
        });

        let msg = RawStdoutMessage::from_value(json);
        match msg {
            RawStdoutMessage::IncomingControlRequest {
                request_id,
                request,
            } => {
                assert_eq!(request_id, "req-1");
                match request {
                    IncomingControlRequestInner::CanUseTool {
                        tool_name,
                        tool_use_id,
                        ..
                    } => {
                        assert_eq!(tool_name, "Bash");
                        assert_eq!(tool_use_id, "tu-1");
                    }
                    _ => panic!("Expected CanUseTool"),
                }
            }
            _ => panic!("Expected IncomingControlRequest"),
        }
    }

    #[test]
    fn parse_control_request_hook_callback() {
        let json = serde_json::json!({
            "type": "control_request",
            "request_id": "req-2",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "cb-1",
                "input": {"hookEventName": "PreToolUse", "toolName": "Bash"},
                "tool_use_id": "tu-2"
            }
        });

        let msg = RawStdoutMessage::from_value(json);
        match msg {
            RawStdoutMessage::IncomingControlRequest {
                request_id,
                request,
            } => {
                assert_eq!(request_id, "req-2");
                match request {
                    IncomingControlRequestInner::HookCallback {
                        callback_id,
                        input,
                        tool_use_id,
                    } => {
                        assert_eq!(callback_id, "cb-1");
                        assert_eq!(input["hookEventName"], "PreToolUse");
                        assert_eq!(tool_use_id, Some("tu-2".to_string()));
                    }
                    _ => panic!("Expected HookCallback"),
                }
            }
            _ => panic!("Expected IncomingControlRequest"),
        }
    }

    #[test]
    fn parse_control_request_mcp_message() {
        let json = serde_json::json!({
            "type": "control_request",
            "request_id": "req-3",
            "request": {
                "subtype": "mcp_message",
                "server_name": "my-server",
                "message": {"jsonrpc": "2.0", "method": "ping"}
            }
        });

        let msg = RawStdoutMessage::from_value(json);
        match msg {
            RawStdoutMessage::IncomingControlRequest {
                request_id,
                request,
            } => {
                assert_eq!(request_id, "req-3");
                match request {
                    IncomingControlRequestInner::McpMessage { server_name, .. } => {
                        assert_eq!(server_name, "my-server");
                    }
                    _ => panic!("Expected McpMessage"),
                }
            }
            _ => panic!("Expected IncomingControlRequest"),
        }
    }

    #[test]
    fn parse_control_response_success() {
        let json = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "sdk-1",
                "response": {"status": "ok"}
            }
        });

        let msg = RawStdoutMessage::from_value(json);
        match msg {
            RawStdoutMessage::ControlResponse(ControlResponseInner::Success {
                request_id,
                response,
            }) => {
                assert_eq!(request_id, "sdk-1");
                assert_eq!(response.unwrap()["status"], "ok");
            }
            _ => panic!("Expected ControlResponse::Success"),
        }
    }

    #[test]
    fn parse_control_response_error() {
        let json = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "error",
                "request_id": "sdk-2",
                "error": "Something failed"
            }
        });

        let msg = RawStdoutMessage::from_value(json);
        match msg {
            RawStdoutMessage::ControlResponse(ControlResponseInner::Error {
                request_id,
                error,
                ..
            }) => {
                assert_eq!(request_id, "sdk-2");
                assert_eq!(error, "Something failed");
            }
            _ => panic!("Expected ControlResponse::Error"),
        }
    }

    #[test]
    fn parse_control_cancel_request() {
        let json = serde_json::json!({
            "type": "control_cancel_request",
            "request_id": "sdk-3"
        });

        let msg = RawStdoutMessage::from_value(json);
        match msg {
            RawStdoutMessage::ControlCancelRequest { request_id } => {
                assert_eq!(request_id, "sdk-3");
            }
            _ => panic!("Expected ControlCancelRequest"),
        }
    }

    #[test]
    fn parse_keep_alive() {
        let json = serde_json::json!({"type": "keep_alive"});
        let msg = RawStdoutMessage::from_value(json);
        assert!(matches!(msg, RawStdoutMessage::KeepAlive));
    }

    #[test]
    fn parse_unknown_type_as_sdk_message_or_unknown() {
        let json = serde_json::json!({
            "type": "assistant",
            "uuid": "msg-1",
            "session_id": "sess-1",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "Hi"}]},
            "parent_tool_use_id": null
        });
        let msg = RawStdoutMessage::from_value(json);
        assert!(matches!(msg, RawStdoutMessage::Sdk(m) if matches!(*m, SdkMessage::Assistant(_))));
    }

    #[test]
    fn parse_invalid_control_request_falls_to_unknown() {
        let json = serde_json::json!({
            "type": "control_request",
            "request_id": "req-bad",
            "request": {
                "subtype": "nonexistent_subtype"
            }
        });

        let msg = RawStdoutMessage::from_value(json);
        assert!(matches!(msg, RawStdoutMessage::Unknown(_)));
    }

    // -- Outgoing control request serialization tests --

    #[test]
    fn serialize_initialize_request() {
        let req = OutgoingControlRequest::Initialize {
            json_schema: None,
            system_prompt: Some("You are helpful.".to_string()),
            append_system_prompt: None,
        };

        let envelope = ControlRequestEnvelope {
            msg_type: "control_request".to_string(),
            request_id: "sdk-1".to_string(),
            request: req,
        };

        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["type"], "control_request");
        assert_eq!(json["request_id"], "sdk-1");
        assert_eq!(json["request"]["subtype"], "initialize");
        assert_eq!(json["request"]["systemPrompt"], "You are helpful.");
    }

    #[test]
    fn serialize_interrupt_request() {
        let req = OutgoingControlRequest::Interrupt;
        let envelope = ControlRequestEnvelope {
            msg_type: "control_request".to_string(),
            request_id: "sdk-2".to_string(),
            request: req,
        };

        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["request"]["subtype"], "interrupt");
    }

    #[test]
    fn serialize_set_model_request() {
        let req = OutgoingControlRequest::SetModel {
            model: Some("claude-sonnet-4-5-20250929".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["subtype"], "set_model");
        assert_eq!(json["model"], "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn serialize_mcp_toggle_request() {
        let req = OutgoingControlRequest::McpToggle {
            server_name: "my-server".to_string(),
            enabled: false,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["subtype"], "mcp_toggle");
        assert_eq!(json["serverName"], "my-server");
        assert_eq!(json["enabled"], false);
    }

    // -- Control response serialization tests --

    #[test]
    fn serialize_control_response_envelope() {
        let envelope = ControlResponseEnvelope {
            msg_type: "control_response".to_string(),
            response: ControlResponsePayload {
                subtype: "success".to_string(),
                request_id: "req-1".to_string(),
                response: Some(serde_json::json!({"behavior": "allow"})),
                error: None,
            },
        };

        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["type"], "control_response");
        assert_eq!(json["response"]["subtype"], "success");
        assert_eq!(json["response"]["request_id"], "req-1");
        assert_eq!(json["response"]["response"]["behavior"], "allow");
        // error should be omitted (skip_serializing_if)
        assert!(json["response"]["error"].is_null());
    }
}
