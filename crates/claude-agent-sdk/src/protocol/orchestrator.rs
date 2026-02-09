use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use crate::error::{Result, SdkError};
use crate::transport::SubprocessTransport;
use crate::types::control::{
    ControlResponseEnvelope, ControlResponseInner, ControlResponsePayload,
    IncomingControlRequestInner, RawStdoutMessage,
};
use crate::types::messages::SdkMessage;
use crate::types::permissions::{PermissionContext, PermissionHandler, PermissionResult};

use super::request_tracker::{ControlErrorInfo, RequestTracker};

/// Spawns a background task that reads from the subprocess transport,
/// routes messages to the consumer channel, and dispatches control requests.
pub(crate) struct Orchestrator;

impl Orchestrator {
    /// Spawn the orchestrator background task.
    ///
    /// Returns a join handle for the background task.
    pub(crate) fn spawn(
        mut transport: SubprocessTransport,
        message_tx: mpsc::Sender<Result<SdkMessage>>,
        request_tracker: Arc<Mutex<RequestTracker>>,
        permission_handler: Option<Arc<dyn PermissionHandler>>,
        stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
        cancellation_token: CancellationToken,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let result = Self::run_loop(
                &mut transport,
                &message_tx,
                &request_tracker,
                permission_handler.as_deref(),
                &stdin,
                &cancellation_token,
            )
            .await;

            if let Err(e) = &result {
                match e {
                    SdkError::Cancelled => {
                        debug!("Orchestrator cancelled");
                    }
                    _ => {
                        error!(error = %e, "Orchestrator error");
                        let _ = message_tx.send(Err(SdkError::Internal(e.to_string()))).await;
                    }
                }
            }

            // Clean up: cancel all pending requests
            {
                let mut tracker = request_tracker.lock().await;
                tracker.cancel_all();
            }

            // Close the transport
            transport.close().await;

            debug!("Orchestrator shutdown complete");
        })
    }

    async fn run_loop(
        transport: &mut SubprocessTransport,
        message_tx: &mpsc::Sender<Result<SdkMessage>>,
        request_tracker: &Arc<Mutex<RequestTracker>>,
        permission_handler: Option<&dyn PermissionHandler>,
        stdin: &Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
        cancellation_token: &CancellationToken,
    ) -> Result<()> {
        loop {
            let msg = tokio::select! {
                result = transport.read_message() => result?,
                () = cancellation_token.cancelled() => {
                    return Err(SdkError::Cancelled);
                }
            };

            let Some(msg) = msg else {
                debug!("Transport EOF — orchestrator exiting");
                return Ok(());
            };

            match msg {
                RawStdoutMessage::Sdk(sdk_msg) => {
                    if message_tx.send(Ok(sdk_msg)).await.is_err() {
                        debug!("Consumer dropped — orchestrator exiting");
                        return Ok(());
                    }
                }
                RawStdoutMessage::IncomingControlRequest {
                    request_id,
                    request,
                } => {
                    Self::handle_incoming_control_request(
                        request_id,
                        request,
                        permission_handler,
                        stdin,
                    )
                    .await;
                }
                RawStdoutMessage::ControlResponse(response) => {
                    Self::handle_control_response(response, request_tracker).await;
                }
                RawStdoutMessage::ControlCancelRequest { request_id } => {
                    let mut tracker = request_tracker.lock().await;
                    if tracker.is_pending(&request_id) {
                        tracker.resolve_error(
                            &request_id,
                            ControlErrorInfo {
                                error: "Request cancelled by CLI".to_string(),
                                pending_permission_requests: None,
                            },
                        );
                    }
                }
                RawStdoutMessage::KeepAlive => {
                    debug!("Received keep-alive");
                }
                RawStdoutMessage::Unknown(value) => {
                    debug!(value = %value, "Unknown message type — ignoring");
                }
            }
        }
    }

    async fn handle_incoming_control_request(
        request_id: String,
        request: IncomingControlRequestInner,
        permission_handler: Option<&dyn PermissionHandler>,
        stdin: &Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
    ) {
        match request {
            IncomingControlRequestInner::CanUseTool {
                tool_name,
                input,
                tool_use_id,
                agent_id,
                permission_suggestions,
                blocked_path,
                decision_reason,
            } => {
                let response = if let Some(handler) = permission_handler {
                    let context = PermissionContext {
                        tool_use_id: tool_use_id.clone(),
                        agent_id,
                        suggestions: permission_suggestions,
                        blocked_path,
                        decision_reason,
                    };

                    match handler.can_use_tool(&tool_name, &input, context).await {
                        PermissionResult::Allow {
                            updated_input,
                            updated_permissions,
                        } => {
                            let mut resp = serde_json::json!({
                                "behavior": "allow",
                                "toolUseID": tool_use_id,
                            });
                            if let Some(input) = updated_input {
                                resp["updatedInput"] =
                                    serde_json::Value::Object(input);
                            }
                            if let Some(perms) = updated_permissions {
                                resp["updatedPermissions"] =
                                    serde_json::to_value(perms).unwrap_or_default();
                            }
                            ControlResponsePayload {
                                subtype: "success".to_string(),
                                request_id: request_id.clone(),
                                response: Some(resp),
                                error: None,
                            }
                        }
                        PermissionResult::Deny { message, interrupt } => {
                            let mut resp = serde_json::json!({
                                "behavior": "deny",
                                "message": message,
                                "toolUseID": tool_use_id,
                            });
                            if let Some(interrupt) = interrupt {
                                resp["interrupt"] = serde_json::Value::Bool(interrupt);
                            }
                            ControlResponsePayload {
                                subtype: "success".to_string(),
                                request_id: request_id.clone(),
                                response: Some(resp),
                                error: None,
                            }
                        }
                    }
                } else {
                    // No permission handler — auto-allow
                    ControlResponsePayload {
                        subtype: "success".to_string(),
                        request_id: request_id.clone(),
                        response: Some(serde_json::json!({
                            "behavior": "allow",
                            "toolUseID": tool_use_id,
                        })),
                        error: None,
                    }
                };

                let envelope = ControlResponseEnvelope {
                    msg_type: "control_response".to_string(),
                    response,
                };

                if let Ok(json) = serde_json::to_string(&envelope) {
                    let mut stdin_guard = stdin.lock().await;
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin_guard.write_all(json.as_bytes()).await;
                    let _ = stdin_guard.write_all(b"\n").await;
                    let _ = stdin_guard.flush().await;
                }
            }
            IncomingControlRequestInner::HookCallback {
                callback_id,
                input: _,
                tool_use_id: _,
            } => {
                // Hook callbacks are not yet implemented — respond with empty success
                debug!(callback_id = %callback_id, "Hook callback received — auto-responding");
                let envelope = ControlResponseEnvelope {
                    msg_type: "control_response".to_string(),
                    response: ControlResponsePayload {
                        subtype: "success".to_string(),
                        request_id: request_id.clone(),
                        response: Some(serde_json::json!({})),
                        error: None,
                    },
                };

                if let Ok(json) = serde_json::to_string(&envelope) {
                    let mut stdin_guard = stdin.lock().await;
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin_guard.write_all(json.as_bytes()).await;
                    let _ = stdin_guard.write_all(b"\n").await;
                    let _ = stdin_guard.flush().await;
                }
            }
            IncomingControlRequestInner::McpMessage {
                server_name,
                message: _,
            } => {
                // MCP message forwarding not yet implemented
                debug!(server_name = %server_name, "MCP message received — ignoring");
            }
        }
    }

    async fn handle_control_response(
        response: ControlResponseInner,
        request_tracker: &Arc<Mutex<RequestTracker>>,
    ) {
        let mut tracker = request_tracker.lock().await;

        match response {
            ControlResponseInner::Success {
                request_id,
                response,
            } => {
                if !tracker.resolve_success(&request_id, response) {
                    warn!(request_id = %request_id, "Received response for unknown request");
                }
            }
            ControlResponseInner::Error {
                request_id,
                error,
                pending_permission_requests,
            } => {
                if !tracker.resolve_error(
                    &request_id,
                    ControlErrorInfo {
                        error,
                        pending_permission_requests,
                    },
                ) {
                    warn!(request_id = %request_id, "Received error response for unknown request");
                }
            }
        }
    }
}
