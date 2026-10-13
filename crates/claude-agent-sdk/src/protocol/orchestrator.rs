use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::api::control::{
    ControlResponseEnvelope, ControlResponseInner, ControlResponsePayload,
    IncomingControlRequestInner, RawStdoutMessage,
};
use crate::api::hooks::{HookHandler, HookOutput};
use crate::api::messages::SdkMessage;
use crate::api::permissions::{PermissionContext, PermissionHandler, PermissionResult};
use crate::error::{Result, SdkError};
use crate::mcp::SdkMcpServer;
use crate::session_cost::SessionCost;
use crate::transport::{StdinWriter, Transport};

use super::request_tracker::{ControlErrorInfo, RequestTracker};

/// Spawns a background task that reads from the subprocess transport, routes
/// messages to the consumer channel, and dispatches control requests.
pub(crate) struct Orchestrator;

impl Orchestrator {
    /// Spawn the orchestrator background task.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn(
        mut transport: Box<dyn Transport>,
        message_tx: mpsc::Sender<Result<SdkMessage>>,
        request_tracker: Arc<Mutex<RequestTracker>>,
        permission_handler: Option<Arc<dyn PermissionHandler>>,
        hook_handler: Option<Arc<dyn HookHandler>>,
        sdk_mcp_servers: Option<HashMap<String, SdkMcpServer>>,
        stdin_writer: Arc<dyn StdinWriter>,
        cancellation_token: CancellationToken,
        mut session_cost: SessionCost,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let result = Self::run_loop(
                &mut *transport,
                &message_tx,
                &request_tracker,
                permission_handler.as_ref(),
                hook_handler.as_deref(),
                sdk_mcp_servers.as_ref(),
                &stdin_writer,
                &cancellation_token,
                &mut session_cost,
            )
            .await;
            session_cost.mark_exiting();

            if let Err(e) = &result {
                if let SdkError::Cancelled = e {
                    debug!("Orchestrator cancelled");
                } else {
                    error!(error = %e, "Orchestrator error");
                    let _ = message_tx
                        .send(Err(SdkError::Internal(e.to_string())))
                        .await;
                }
            }

            {
                let mut tracker = request_tracker.lock().await;
                tracker.cancel_all();
            }

            let shutdown_reason = match &result {
                Ok(()) => "completed",
                Err(SdkError::Cancelled) => "cancelled",
                Err(_) => "error",
            };

            transport.close().await;
            // Only now has the CLI written the `cost-state` a resume restores.
            drop(session_cost);

            debug!(reason = shutdown_reason, "Orchestrator shutdown complete");
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_loop(
        transport: &mut dyn Transport,
        message_tx: &mpsc::Sender<Result<SdkMessage>>,
        request_tracker: &Arc<Mutex<RequestTracker>>,
        permission_handler: Option<&Arc<dyn PermissionHandler>>,
        hook_handler: Option<&dyn HookHandler>,
        sdk_mcp_servers: Option<&HashMap<String, SdkMcpServer>>,
        stdin_writer: &Arc<dyn StdinWriter>,
        cancellation_token: &CancellationToken,
        session_cost: &mut SessionCost,
    ) -> Result<()> {
        loop {
            let msg = tokio::select! {
                result = transport.read_message() => result?,
                () = cancellation_token.cancelled() => {
                    return Err(SdkError::Cancelled);
                }
            };

            let Some(msg) = msg else {
                info!("Transport EOF — orchestrator exiting");
                return Ok(());
            };

            match msg {
                RawStdoutMessage::Sdk(sdk_msg) => {
                    let mut sdk_msg = *sdk_msg;
                    session_cost.observe(&mut sdk_msg);
                    let is_result = matches!(&sdk_msg, SdkMessage::Result(_));
                    if message_tx.send(Ok(sdk_msg)).await.is_err() {
                        warn!("Consumer dropped — orchestrator exiting");
                        return Ok(());
                    }
                    if is_result {
                        info!("Result message delivered — ending message stream");
                        return Ok(()); // Drops message_tx → recv() returns None naturally
                    }
                }
                RawStdoutMessage::IncomingControlRequest {
                    request_id,
                    request,
                } => match request {
                    IncomingControlRequestInner::CanUseTool {
                        tool_name,
                        input,
                        tool_use_id,
                        agent_id,
                        permission_suggestions,
                        blocked_path,
                        decision_reason,
                    } => {
                        // Spawned, not awaited: a decision can take as long as
                        // the person deciding, and awaiting here would stall
                        // the only stdout reader, leaving already-written
                        // messages — including the call being asked about —
                        // unread until answered.
                        let context = PermissionContext {
                            tool_use_id: tool_use_id.clone(),
                            agent_id,
                            suggestions: permission_suggestions,
                            blocked_path,
                            decision_reason,
                        };
                        let permission_handler = permission_handler.cloned();
                        let stdin_writer = Arc::clone(stdin_writer);
                        let cancellation_token = cancellation_token.clone();
                        tokio::spawn(async move {
                            tokio::select! {
                                () = Self::answer_can_use_tool(
                                    request_id,
                                    &tool_name,
                                    input,
                                    context,
                                    permission_handler.as_deref(),
                                    &stdin_writer,
                                ) => {}
                                () = cancellation_token.cancelled() => {
                                    debug!(
                                        tool_name = %tool_name,
                                        "Cancelled while a permission decision was outstanding \
                                         — leaving it unanswered"
                                    );
                                }
                            }
                        });
                    }
                    request => {
                        Self::handle_incoming_control_request(
                            request_id,
                            request,
                            hook_handler,
                            sdk_mcp_servers,
                            stdin_writer,
                        )
                        .await;
                    }
                },
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

    /// Put one `can_use_tool` request to the handler and write its answer.
    ///
    /// Runs off the read loop: the handler may take arbitrarily long, including
    /// waiting on a person.
    async fn answer_can_use_tool(
        request_id: String,
        tool_name: &str,
        input: serde_json::Map<String, serde_json::Value>,
        context: PermissionContext,
        permission_handler: Option<&dyn PermissionHandler>,
        stdin_writer: &Arc<dyn StdinWriter>,
    ) {
        let tool_use_id = context.tool_use_id.clone();
        let response = if let Some(handler) = permission_handler {
            match handler.can_use_tool(tool_name, &input, context).await {
                PermissionResult::Allow {
                    updated_input,
                    updated_permissions,
                } => {
                    // CLI's Zod schema requires `updatedInput` to be a record
                    // for `behavior: "allow"` responses. Use the updated input
                    // if provided, otherwise fall back to the original input.
                    let input_value = match updated_input {
                        Some(map) => serde_json::Value::Object(map),
                        None => serde_json::Value::Object(input),
                    };
                    let mut resp = serde_json::json!({
                        "behavior": "allow",
                        "toolUseID": tool_use_id,
                        "updatedInput": input_value,
                    });
                    if let Some(perms) = updated_permissions {
                        resp["updatedPermissions"] =
                            serde_json::to_value(perms).unwrap_or_default();
                    }
                    ControlResponsePayload {
                        subtype: "success".to_string(),
                        request_id,
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
                        request_id,
                        response: Some(resp),
                        error: None,
                    }
                }
            }
        } else {
            // No permission handler — auto-allow with original input
            ControlResponsePayload {
                subtype: "success".to_string(),
                request_id,
                response: Some(serde_json::json!({
                    "behavior": "allow",
                    "toolUseID": tool_use_id,
                    "updatedInput": serde_json::Value::Object(input),
                })),
                error: None,
            }
        };

        Self::write_control_response(response, stdin_writer).await;
    }

    /// Handle a control request the read loop answers in line, in the order the
    /// CLI asked. `can_use_tool` is not one of them — it is spawned; see
    /// [`Self::answer_can_use_tool`].
    async fn handle_incoming_control_request(
        request_id: String,
        request: IncomingControlRequestInner,
        hook_handler: Option<&dyn HookHandler>,
        sdk_mcp_servers: Option<&HashMap<String, SdkMcpServer>>,
        stdin_writer: &Arc<dyn StdinWriter>,
    ) {
        match request {
            IncomingControlRequestInner::CanUseTool { .. } => {
                warn!("can_use_tool reached the in-line control path — ignoring");
            }
            IncomingControlRequestInner::HookCallback {
                callback_id,
                input,
                tool_use_id,
            } => {
                let response_value = if let Some(handler) = hook_handler {
                    debug!(callback_id = %callback_id, "Hook callback received — invoking handler");
                    let output = handler.on_hook(&input, tool_use_id.as_deref()).await;
                    match output {
                        HookOutput::Async(async_output) => {
                            serde_json::to_value(async_output).unwrap_or_default()
                        }
                        HookOutput::Sync(sync_output) => {
                            serde_json::to_value(sync_output).unwrap_or_default()
                        }
                    }
                } else {
                    // No hook handler — auto-respond with empty success (continue)
                    debug!(callback_id = %callback_id, "Hook callback received — auto-responding (no handler)");
                    serde_json::json!({})
                };

                let response = ControlResponsePayload {
                    subtype: "success".to_string(),
                    request_id: request_id.clone(),
                    response: Some(response_value),
                    error: None,
                };

                Self::write_control_response(response, stdin_writer).await;
            }
            IncomingControlRequestInner::McpMessage {
                server_name,
                message,
            } => {
                if let Some(servers) = sdk_mcp_servers {
                    if let Some(server) = servers.get(&server_name) {
                        debug!(server_name = %server_name, "MCP message received — routing to SDK server");
                        let mcp_response = server.handle_message(&message).await;

                        // Wrap as { mcp_response: <jsonrpc_response> } per the protocol
                        let response_payload = if mcp_response.is_null() {
                            serde_json::json!({ "mcp_response": { "jsonrpc": "2.0", "result": {}, "id": 0 } })
                        } else {
                            serde_json::json!({ "mcp_response": mcp_response })
                        };

                        let response = ControlResponsePayload {
                            subtype: "success".to_string(),
                            request_id: request_id.clone(),
                            response: Some(response_payload),
                            error: None,
                        };
                        Self::write_control_response(response, stdin_writer).await;
                    } else {
                        warn!(server_name = %server_name, "SDK MCP server not found");
                        let response = ControlResponsePayload {
                            subtype: "error".to_string(),
                            request_id: request_id.clone(),
                            response: None,
                            error: Some(format!("SDK MCP server '{server_name}' not found")),
                        };
                        Self::write_control_response(response, stdin_writer).await;
                    }
                } else {
                    debug!(server_name = %server_name, "MCP message received — no SDK servers registered");
                }
            }
        }
    }

    /// Write a control response envelope to stdin.
    async fn write_control_response(
        response: ControlResponsePayload,
        stdin_writer: &Arc<dyn StdinWriter>,
    ) {
        let envelope = ControlResponseEnvelope {
            msg_type: "control_response".to_string(),
            response,
        };

        if let Ok(json) = serde_json::to_string(&envelope) {
            let _ = stdin_writer.write_json_line(&json).await;
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
