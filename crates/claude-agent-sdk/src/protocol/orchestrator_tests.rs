#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex};
    use tokio_util::sync::CancellationToken;

    use crate::error::Result;
    use crate::mcp::SdkMcpServer;
    use crate::protocol::orchestrator::Orchestrator;
    use crate::protocol::request_tracker::RequestTracker;
    use crate::transport::mock::{MockStdinWriter, MockTransport};
    use crate::types::control::{
        ControlResponseInner, IncomingControlRequestInner, RawStdoutMessage,
    };
    use crate::types::hooks::{HookHandler, HookOutput, SyncHookOutput};
    use crate::types::messages::SdkMessage;
    use crate::types::permissions::{
        PermissionContext, PermissionHandler, PermissionResult, PermissionUpdate,
    };

    /// Helper to spawn an orchestrator with mock transport and return the pieces.
    struct TestHarness {
        /// Send messages to the orchestrator (simulates CLI stdout)
        msg_feeder: mpsc::Sender<RawStdoutMessage>,
        /// Receive SDK messages from the orchestrator (consumer side)
        sdk_rx: mpsc::Receiver<Result<SdkMessage>>,
        /// Receive JSON lines written by the orchestrator (captures stdin writes)
        stdin_rx: mpsc::UnboundedReceiver<String>,
        /// Request tracker for outgoing control requests
        request_tracker: Arc<Mutex<RequestTracker>>,
        /// Cancellation token
        cancel: CancellationToken,
    }

    impl TestHarness {
        fn new(
            permission_handler: Option<Arc<dyn PermissionHandler>>,
            hook_handler: Option<Arc<dyn HookHandler>>,
        ) -> Self {
            Self::with_mcp_servers(permission_handler, hook_handler, None)
        }

        fn with_mcp_servers(
            permission_handler: Option<Arc<dyn PermissionHandler>>,
            hook_handler: Option<Arc<dyn HookHandler>>,
            sdk_mcp_servers: Option<HashMap<String, SdkMcpServer>>,
        ) -> Self {
            let (msg_tx, msg_rx) = mpsc::channel(64);
            let (sdk_tx, sdk_rx) = mpsc::channel(64);
            let (stdin_writer, stdin_rx) = MockStdinWriter::new();
            let request_tracker = Arc::new(Mutex::new(RequestTracker::new()));
            let cancel = CancellationToken::new();

            let transport = MockTransport::new(msg_rx);

            Orchestrator::spawn(
                Box::new(transport),
                sdk_tx,
                request_tracker.clone(),
                permission_handler,
                hook_handler,
                sdk_mcp_servers,
                Arc::new(stdin_writer),
                cancel.clone(),
            );

            Self {
                msg_feeder: msg_tx,
                sdk_rx,
                stdin_rx,
                request_tracker,
                cancel,
            }
        }

        /// Send a raw message as if it came from the CLI's stdout.
        async fn feed(&self, msg: RawStdoutMessage) {
            self.msg_feeder.send(msg).await.unwrap();
        }

        /// Receive the next SDK message from the consumer channel.
        async fn recv(&mut self) -> SdkMessage {
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                self.sdk_rx.recv(),
            )
            .await
            {
                Ok(Some(Ok(msg))) => msg,
                Ok(Some(Err(e))) => panic!("Received error: {e}"),
                Ok(None) => panic!("Channel closed"),
                Err(_) => panic!("Timeout waiting for SDK message"),
            }
        }

        /// Read the next JSON line written to stdin (control response).
        async fn read_stdin_line(&mut self) -> serde_json::Value {
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                self.stdin_rx.recv(),
            )
            .await
            {
                Ok(Some(line)) => serde_json::from_str(&line).unwrap(),
                Ok(None) => panic!("Stdin channel closed"),
                Err(_) => panic!("Timeout waiting for stdin write"),
            }
        }
    }

    impl Drop for TestHarness {
        fn drop(&mut self) {
            self.cancel.cancel();
        }
    }

    // ============================================================
    // SDK Message Forwarding Tests
    // ============================================================

    #[tokio::test]
    async fn forwards_assistant_message_to_consumer() {
        let mut h = TestHarness::new(None, None);

        let assistant_json = serde_json::json!({
            "type": "assistant",
            "uuid": "msg-1",
            "session_id": "sess-1",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello!"}]
            },
            "parent_tool_use_id": null
        });
        let sdk_msg = SdkMessage::from_value(assistant_json).unwrap();
        h.feed(RawStdoutMessage::Sdk(sdk_msg)).await;

        let msg = h.recv().await;
        assert!(matches!(msg, SdkMessage::Assistant(_)));
    }

    #[tokio::test]
    async fn result_message_ends_stream() {
        let mut h = TestHarness::new(None, None);

        let result_json = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "uuid": "msg-1",
            "session_id": "sess-1",
            "duration_ms": 100,
            "duration_api_ms": 80,
            "is_error": false,
            "num_turns": 1,
            "result": "Done",
            "total_cost_usd": 0.01,
            "usage": {"input_tokens": 10, "output_tokens": 5},
            "modelUsage": {},
            "permission_denials": []
        });
        let sdk_msg = SdkMessage::from_value(result_json).unwrap();
        h.feed(RawStdoutMessage::Sdk(sdk_msg)).await;

        let msg = h.recv().await;
        assert!(matches!(msg, SdkMessage::Result(_)));

        // Channel should close after result
        let next = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            h.sdk_rx.recv(),
        )
        .await;
        assert!(next.is_ok()); // Returns Ok(None) — channel closed
        assert!(next.unwrap().is_none());
    }

    #[tokio::test]
    async fn keepalive_does_not_forward() {
        let mut h = TestHarness::new(None, None);

        h.feed(RawStdoutMessage::KeepAlive).await;

        // No message should arrive
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            h.sdk_rx.recv(),
        )
        .await;
        assert!(result.is_err()); // Timeout — no message forwarded
    }

    #[tokio::test]
    async fn unknown_message_does_not_forward() {
        let mut h = TestHarness::new(None, None);

        h.feed(RawStdoutMessage::Unknown(serde_json::json!({"type": "future_type"}))).await;

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            h.sdk_rx.recv(),
        )
        .await;
        assert!(result.is_err()); // Timeout — unknown messages are ignored
    }

    // ============================================================
    // Permission Handler Tests
    // ============================================================

    struct AllowAllHandler;

    #[async_trait::async_trait]
    impl PermissionHandler for AllowAllHandler {
        async fn can_use_tool(
            &self,
            _tool_name: &str,
            _input: &serde_json::Map<String, serde_json::Value>,
            _context: PermissionContext,
        ) -> PermissionResult {
            PermissionResult::Allow {
                updated_input: None,
                updated_permissions: None,
            }
        }
    }

    struct DenyAllHandler {
        message: String,
    }

    #[async_trait::async_trait]
    impl PermissionHandler for DenyAllHandler {
        async fn can_use_tool(
            &self,
            _tool_name: &str,
            _input: &serde_json::Map<String, serde_json::Value>,
            _context: PermissionContext,
        ) -> PermissionResult {
            PermissionResult::Deny {
                message: self.message.clone(),
                interrupt: Some(true),
            }
        }
    }

    /// A handler that captures the tool name and input for inspection.
    struct CapturingHandler {
        captured: Arc<Mutex<Vec<(String, serde_json::Map<String, serde_json::Value>)>>>,
    }

    #[async_trait::async_trait]
    impl PermissionHandler for CapturingHandler {
        async fn can_use_tool(
            &self,
            tool_name: &str,
            input: &serde_json::Map<String, serde_json::Value>,
            _context: PermissionContext,
        ) -> PermissionResult {
            let mut guard = self.captured.lock().await;
            guard.push((tool_name.to_string(), input.clone()));
            PermissionResult::Allow {
                updated_input: None,
                updated_permissions: None,
            }
        }
    }

    fn make_can_use_tool_request(
        request_id: &str,
        tool_name: &str,
        tool_use_id: &str,
    ) -> RawStdoutMessage {
        RawStdoutMessage::IncomingControlRequest {
            request_id: request_id.to_string(),
            request: IncomingControlRequestInner::CanUseTool {
                tool_name: tool_name.to_string(),
                input: serde_json::Map::new(),
                tool_use_id: tool_use_id.to_string(),
                agent_id: None,
                permission_suggestions: None,
                blocked_path: None,
                decision_reason: None,
            },
        }
    }

    #[tokio::test]
    async fn permission_handler_allow_sends_allow_response() {
        let mut h = TestHarness::new(Some(Arc::new(AllowAllHandler)), None);

        h.feed(make_can_use_tool_request("req-1", "Bash", "tu-1")).await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");
        assert_eq!(resp["request_id"], "req-1");
        assert_eq!(resp["response"]["behavior"], "allow");
        assert_eq!(resp["response"]["toolUseID"], "tu-1");
        // updatedInput must always be present (CLI Zod schema requires it)
        assert!(resp["response"]["updatedInput"].is_object());
    }

    #[tokio::test]
    async fn permission_handler_deny_sends_deny_response() {
        let handler = DenyAllHandler {
            message: "Not allowed".to_string(),
        };
        let mut h = TestHarness::new(Some(Arc::new(handler)), None);

        h.feed(make_can_use_tool_request("req-2", "Write", "tu-2")).await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");
        assert_eq!(resp["response"]["behavior"], "deny");
        assert_eq!(resp["response"]["message"], "Not allowed");
        assert_eq!(resp["response"]["interrupt"], true);
    }

    #[tokio::test]
    async fn no_permission_handler_auto_allows() {
        let mut h = TestHarness::new(None, None);

        h.feed(make_can_use_tool_request("req-3", "Read", "tu-3")).await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["response"]["behavior"], "allow");
        assert_eq!(resp["response"]["toolUseID"], "tu-3");
        // updatedInput must always be present (CLI Zod schema requires it)
        assert!(resp["response"]["updatedInput"].is_object());
    }

    #[tokio::test]
    async fn permission_handler_receives_correct_tool_info() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let handler = CapturingHandler {
            captured: captured.clone(),
        };
        let mut h = TestHarness::new(Some(Arc::new(handler)), None);

        let mut input = serde_json::Map::new();
        input.insert("command".to_string(), serde_json::json!("ls -la"));

        h.feed(RawStdoutMessage::IncomingControlRequest {
            request_id: "req-4".to_string(),
            request: IncomingControlRequestInner::CanUseTool {
                tool_name: "Bash".to_string(),
                input: input.clone(),
                tool_use_id: "tu-4".to_string(),
                agent_id: Some("agent-1".to_string()),
                permission_suggestions: None,
                blocked_path: None,
                decision_reason: Some("requires approval".to_string()),
            },
        })
        .await;

        // Wait for the response to be written
        let _ = h.read_stdin_line().await;

        let guard = captured.lock().await;
        assert_eq!(guard.len(), 1);
        assert_eq!(guard[0].0, "Bash");
        assert_eq!(guard[0].1["command"], "ls -la");
    }

    #[tokio::test]
    async fn multiple_permission_requests_handled_sequentially() {
        let mut h = TestHarness::new(Some(Arc::new(AllowAllHandler)), None);

        h.feed(make_can_use_tool_request("req-a", "Read", "tu-a")).await;
        h.feed(make_can_use_tool_request("req-b", "Write", "tu-b")).await;

        let resp_a = h.read_stdin_line().await;
        let resp_b = h.read_stdin_line().await;

        assert_eq!(resp_a["response"]["request_id"], "req-a");
        assert_eq!(resp_b["response"]["request_id"], "req-b");
    }

    // ============================================================
    // Hook Handler Tests
    // ============================================================

    struct ApprovingHookHandler;

    #[async_trait::async_trait]
    impl HookHandler for ApprovingHookHandler {
        async fn on_hook(
            &self,
            _input: &serde_json::Value,
            _tool_use_id: Option<&str>,
        ) -> HookOutput {
            HookOutput::Sync(SyncHookOutput {
                should_continue: Some(true),
                decision: Some("approve".to_string()),
                ..Default::default()
            })
        }
    }

    struct DenyingHookHandler {
        reason: String,
    }

    #[async_trait::async_trait]
    impl HookHandler for DenyingHookHandler {
        async fn on_hook(
            &self,
            _input: &serde_json::Value,
            _tool_use_id: Option<&str>,
        ) -> HookOutput {
            HookOutput::Sync(SyncHookOutput {
                should_continue: Some(false),
                stop_reason: Some(self.reason.clone()),
                decision: Some("deny".to_string()),
                ..Default::default()
            })
        }
    }

    struct SystemMessageHookHandler {
        message: String,
    }

    #[async_trait::async_trait]
    impl HookHandler for SystemMessageHookHandler {
        async fn on_hook(
            &self,
            _input: &serde_json::Value,
            _tool_use_id: Option<&str>,
        ) -> HookOutput {
            HookOutput::Sync(SyncHookOutput {
                should_continue: Some(true),
                system_message: Some(self.message.clone()),
                ..Default::default()
            })
        }
    }

    /// A hook handler that captures the input for inspection.
    struct CapturingHookHandler {
        captured: Arc<Mutex<Vec<(serde_json::Value, Option<String>)>>>,
    }

    #[async_trait::async_trait]
    impl HookHandler for CapturingHookHandler {
        async fn on_hook(
            &self,
            input: &serde_json::Value,
            tool_use_id: Option<&str>,
        ) -> HookOutput {
            let mut guard = self.captured.lock().await;
            guard.push((input.clone(), tool_use_id.map(String::from)));
            HookOutput::Sync(SyncHookOutput::default())
        }
    }

    fn make_hook_callback_request(
        request_id: &str,
        callback_id: &str,
        event_name: &str,
        tool_use_id: Option<&str>,
    ) -> RawStdoutMessage {
        RawStdoutMessage::IncomingControlRequest {
            request_id: request_id.to_string(),
            request: IncomingControlRequestInner::HookCallback {
                callback_id: callback_id.to_string(),
                input: serde_json::json!({
                    "hookEventName": event_name,
                    "toolName": "Bash",
                    "toolInput": {"command": "echo hello"}
                }),
                tool_use_id: tool_use_id.map(String::from),
            },
        }
    }

    #[tokio::test]
    async fn hook_handler_approve_sends_continue_response() {
        let mut h = TestHarness::new(None, Some(Arc::new(ApprovingHookHandler)));

        h.feed(make_hook_callback_request("req-h1", "cb-1", "PreToolUse", Some("tu-1")))
            .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");
        assert_eq!(resp["request_id"], "req-h1");
        assert_eq!(resp["response"]["continue"], true);
        assert_eq!(resp["response"]["decision"], "approve");
    }

    #[tokio::test]
    async fn hook_handler_deny_sends_stop_response() {
        let handler = DenyingHookHandler {
            reason: "Dangerous command".to_string(),
        };
        let mut h = TestHarness::new(None, Some(Arc::new(handler)));

        h.feed(make_hook_callback_request("req-h2", "cb-2", "PreToolUse", Some("tu-2")))
            .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["response"]["continue"], false);
        assert_eq!(resp["response"]["stopReason"], "Dangerous command");
        assert_eq!(resp["response"]["decision"], "deny");
    }

    #[tokio::test]
    async fn hook_handler_system_message_included() {
        let handler = SystemMessageHookHandler {
            message: "Remember to check permissions".to_string(),
        };
        let mut h = TestHarness::new(None, Some(Arc::new(handler)));

        h.feed(make_hook_callback_request("req-h3", "cb-3", "PostToolUse", None))
            .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(
            resp["response"]["systemMessage"],
            "Remember to check permissions"
        );
    }

    #[tokio::test]
    async fn no_hook_handler_auto_responds_empty() {
        let mut h = TestHarness::new(None, None);

        h.feed(make_hook_callback_request("req-h4", "cb-4", "PreToolUse", Some("tu-4")))
            .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");
        // Empty response object — no continue, no decision
        let resp_value = resp["response"].as_object().unwrap();
        assert!(resp_value.is_empty());
    }

    #[tokio::test]
    async fn hook_handler_receives_correct_input() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let handler = CapturingHookHandler {
            captured: captured.clone(),
        };
        let mut h = TestHarness::new(None, Some(Arc::new(handler)));

        h.feed(make_hook_callback_request("req-h5", "cb-5", "PreToolUse", Some("tu-5")))
            .await;

        // Wait for the response
        let _ = h.read_stdin_line().await;

        let guard = captured.lock().await;
        assert_eq!(guard.len(), 1);
        assert_eq!(guard[0].0["hookEventName"], "PreToolUse");
        assert_eq!(guard[0].0["toolName"], "Bash");
        assert_eq!(guard[0].1, Some("tu-5".to_string()));
    }

    #[tokio::test]
    async fn hook_and_permission_handlers_coexist() {
        let mut h = TestHarness::new(
            Some(Arc::new(AllowAllHandler)),
            Some(Arc::new(ApprovingHookHandler)),
        );

        // Send a permission request
        h.feed(make_can_use_tool_request("req-p1", "Bash", "tu-p1")).await;
        let perm_resp = h.read_stdin_line().await;
        assert_eq!(perm_resp["response"]["response"]["behavior"], "allow");

        // Send a hook callback
        h.feed(make_hook_callback_request("req-hh1", "cb-hh1", "PreToolUse", Some("tu-hh1")))
            .await;
        let hook_resp = h.read_stdin_line().await;
        assert_eq!(hook_resp["response"]["response"]["continue"], true);
    }

    // ============================================================
    // Control Response Routing Tests
    // ============================================================

    #[tokio::test]
    async fn control_response_resolves_pending_request() {
        let h = TestHarness::new(None, None);

        // Register a pending request
        let (req_id, rx) = {
            let mut tracker = h.request_tracker.lock().await;
            tracker.register()
        };

        // Feed a success response
        h.feed(RawStdoutMessage::ControlResponse(
            ControlResponseInner::Success {
                request_id: req_id.clone(),
                response: Some(serde_json::json!({"status": "ok"})),
            },
        ))
        .await;

        // The receiver should get the response
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn control_error_response_resolves_as_error() {
        let h = TestHarness::new(None, None);

        let (req_id, rx) = {
            let mut tracker = h.request_tracker.lock().await;
            tracker.register()
        };

        h.feed(RawStdoutMessage::ControlResponse(
            ControlResponseInner::Error {
                request_id: req_id.clone(),
                error: "Something went wrong".to_string(),
                pending_permission_requests: None,
            },
        ))
        .await;

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error, "Something went wrong");
    }

    #[tokio::test]
    async fn control_cancel_request_resolves_as_error() {
        let h = TestHarness::new(None, None);

        let (req_id, rx) = {
            let mut tracker = h.request_tracker.lock().await;
            tracker.register()
        };

        h.feed(RawStdoutMessage::ControlCancelRequest {
            request_id: req_id.clone(),
        })
        .await;

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.error.contains("cancelled"));
    }

    // ============================================================
    // Cancellation Tests
    // ============================================================

    #[tokio::test]
    async fn cancellation_stops_orchestrator() {
        let mut h = TestHarness::new(None, None);

        // Feed a message first to prove it's working
        let json = serde_json::json!({
            "type": "assistant",
            "uuid": "msg-1",
            "session_id": "sess-1",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello"}]
            },
            "parent_tool_use_id": null
        });
        h.feed(RawStdoutMessage::Sdk(SdkMessage::from_value(json).unwrap()))
            .await;
        let _ = h.recv().await;

        // Cancel
        h.cancel.cancel();

        // Channel should close
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            h.sdk_rx.recv(),
        )
        .await;
        // Either we get an error message or the channel closes
        match result {
            Ok(Some(Err(_))) => {} // Cancelled error
            Ok(None) => {}          // Channel closed
            other => panic!("Unexpected result: {other:?}"),
        }
    }

    // ============================================================
    // Transport EOF Tests
    // ============================================================

    #[tokio::test]
    async fn transport_eof_closes_stream() {
        // Build manually so we can drop the feeder
        let (msg_tx, msg_rx) = mpsc::channel(64);
        let (sdk_tx, mut sdk_rx) = mpsc::channel(64);
        let (stdin_writer, _stdin_rx) = MockStdinWriter::new();
        let request_tracker = Arc::new(Mutex::new(RequestTracker::new()));
        let cancel = CancellationToken::new();

        let transport = MockTransport::new(msg_rx);
        Orchestrator::spawn(
            Box::new(transport),
            sdk_tx,
            request_tracker,
            None,
            None,
            None,
            Arc::new(stdin_writer),
            cancel.clone(),
        );

        // Drop the feeder to signal EOF
        drop(msg_tx);

        // Channel should close gracefully
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            sdk_rx.recv(),
        )
        .await;
        assert!(matches!(result, Ok(None)));

        cancel.cancel();
    }

    // ============================================================
    // Permission Handler with Updated Input/Permissions
    // ============================================================

    struct InputModifyingHandler;

    #[async_trait::async_trait]
    impl PermissionHandler for InputModifyingHandler {
        async fn can_use_tool(
            &self,
            _tool_name: &str,
            _input: &serde_json::Map<String, serde_json::Value>,
            _context: PermissionContext,
        ) -> PermissionResult {
            let mut updated = serde_json::Map::new();
            updated.insert("command".to_string(), serde_json::json!("echo safe"));
            PermissionResult::Allow {
                updated_input: Some(updated),
                updated_permissions: Some(vec![PermissionUpdate::AddRules {
                    rules: vec![],
                    behavior: crate::types::permissions::PermissionBehavior::Allow,
                    destination:
                        crate::types::permissions::PermissionUpdateDestination::Session,
                }]),
            }
        }
    }

    #[tokio::test]
    async fn permission_handler_allow_with_updated_input() {
        let mut h = TestHarness::new(Some(Arc::new(InputModifyingHandler)), None);

        h.feed(make_can_use_tool_request("req-mod", "Bash", "tu-mod")).await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"]["response"];
        assert_eq!(resp["behavior"], "allow");
        assert_eq!(resp["updatedInput"]["command"], "echo safe");
        assert!(resp["updatedPermissions"].is_array());
    }

    // ============================================================
    // SDK MCP Server Routing Tests
    // ============================================================

    use crate::mcp::{SdkMcpTool, ToolResult};

    fn build_test_mcp_server() -> SdkMcpServer {
        SdkMcpServer::builder("test-server")
            .tool(
                SdkMcpTool::new("add", "Add two numbers")
                    .input_schema(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "a": {"type": "number"},
                            "b": {"type": "number"}
                        },
                        "required": ["a", "b"]
                    }))
                    .handler(|args| {
                        Box::pin(async move {
                            let a = args["a"].as_f64().unwrap_or(0.0);
                            let b = args["b"].as_f64().unwrap_or(0.0);
                            ToolResult::text(format!("{}", a + b))
                        })
                    }),
            )
            .build()
    }

    fn make_mcp_message_request(
        request_id: &str,
        server_name: &str,
        message: serde_json::Value,
    ) -> RawStdoutMessage {
        RawStdoutMessage::IncomingControlRequest {
            request_id: request_id.to_string(),
            request: IncomingControlRequestInner::McpMessage {
                server_name: server_name.to_string(),
                message,
            },
        }
    }

    #[tokio::test]
    async fn mcp_tools_call_routes_to_server() {
        let mut servers = HashMap::new();
        servers.insert("test-server".to_string(), build_test_mcp_server());

        let mut h = TestHarness::with_mcp_servers(None, None, Some(servers));

        h.feed(make_mcp_message_request(
            "req-mcp-1",
            "test-server",
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "tools/call",
                "params": {
                    "name": "add",
                    "arguments": {"a": 3, "b": 4}
                },
                "id": 1
            }),
        ))
        .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");
        assert_eq!(resp["request_id"], "req-mcp-1");

        // Response is wrapped as { mcp_response: <jsonrpc_response> }
        let mcp_response = &resp["response"]["mcp_response"];
        let tool_result = &mcp_response["result"]["content"][0];
        assert_eq!(tool_result["type"], "text");
        assert_eq!(tool_result["text"], "7");
    }

    #[tokio::test]
    async fn mcp_tools_list_routes_to_server() {
        let mut servers = HashMap::new();
        servers.insert("test-server".to_string(), build_test_mcp_server());

        let mut h = TestHarness::with_mcp_servers(None, None, Some(servers));

        h.feed(make_mcp_message_request(
            "req-mcp-2",
            "test-server",
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "tools/list",
                "id": 2
            }),
        ))
        .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");

        let mcp_response = &resp["response"]["mcp_response"];
        let tools = mcp_response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "add");
    }

    #[tokio::test]
    async fn mcp_initialize_routes_to_server() {
        let mut servers = HashMap::new();
        servers.insert("test-server".to_string(), build_test_mcp_server());

        let mut h = TestHarness::with_mcp_servers(None, None, Some(servers));

        h.feed(make_mcp_message_request(
            "req-mcp-3",
            "test-server",
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "initialize",
                "params": {},
                "id": 3
            }),
        ))
        .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");

        let mcp_response = &resp["response"]["mcp_response"];
        assert_eq!(mcp_response["result"]["serverInfo"]["name"], "test-server");
    }

    #[tokio::test]
    async fn mcp_notification_returns_dummy_response() {
        let mut servers = HashMap::new();
        servers.insert("test-server".to_string(), build_test_mcp_server());

        let mut h = TestHarness::with_mcp_servers(None, None, Some(servers));

        h.feed(make_mcp_message_request(
            "req-mcp-4",
            "test-server",
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        ))
        .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");

        // Notifications return a dummy mcp_response since null is not valid
        let mcp_response = &resp["response"]["mcp_response"];
        assert!(mcp_response.is_object());
    }

    #[tokio::test]
    async fn mcp_unknown_server_returns_error() {
        let mut servers = HashMap::new();
        servers.insert("test-server".to_string(), build_test_mcp_server());

        let mut h = TestHarness::with_mcp_servers(None, None, Some(servers));

        h.feed(make_mcp_message_request(
            "req-mcp-5",
            "nonexistent-server",
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "ping",
                "id": 5
            }),
        ))
        .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "error");
        assert_eq!(resp["request_id"], "req-mcp-5");
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("nonexistent-server"));
    }

    #[tokio::test]
    async fn mcp_no_servers_registered_ignores_message() {
        let mut h = TestHarness::new(None, None);

        h.feed(make_mcp_message_request(
            "req-mcp-6",
            "some-server",
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "ping",
                "id": 6
            }),
        ))
        .await;

        // No response should be written (no servers registered)
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            h.stdin_rx.recv(),
        )
        .await;
        assert!(result.is_err()); // Timeout — no response written
    }

    #[tokio::test]
    async fn mcp_and_permissions_coexist() {
        let mut servers = HashMap::new();
        servers.insert("calc".to_string(), build_test_mcp_server());

        let mut h = TestHarness::with_mcp_servers(
            Some(Arc::new(AllowAllHandler)),
            None,
            Some(servers),
        );

        // Send permission request
        h.feed(make_can_use_tool_request("req-p1", "Bash", "tu-p1")).await;
        let perm_resp = h.read_stdin_line().await;
        assert_eq!(perm_resp["response"]["response"]["behavior"], "allow");

        // Send MCP message
        h.feed(make_mcp_message_request(
            "req-mcp-7",
            "calc",
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "ping",
                "id": 7
            }),
        ))
        .await;
        let mcp_resp = h.read_stdin_line().await;
        assert_eq!(mcp_resp["response"]["subtype"], "success");
        let mcp_response = &mcp_resp["response"]["response"]["mcp_response"];
        assert!(mcp_response.is_object());
    }
}
