#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tokio::sync::{Mutex, mpsc};
    use tokio_util::sync::CancellationToken;

    use crate::api::control::{
        ControlResponseInner, IncomingControlRequestInner, RawStdoutMessage,
    };
    use crate::api::messages::SdkMessage;
    use crate::api::permissions::{
        PermissionContext, PermissionHandler, PermissionResult, PermissionUpdate,
    };
    use crate::error::Result;
    use crate::protocol::orchestrator::Orchestrator;
    use crate::protocol::request_tracker::RequestTracker;
    use crate::session_cost::SessionCost;
    use crate::transport::mock::{MockStdinWriter, MockTransport};

    /// Helper to spawn an orchestrator with mock transport and return the
    /// pieces.
    struct TestHarness {
        /// Send messages to the orchestrator (simulates CLI stdout)
        msg_feeder: mpsc::Sender<RawStdoutMessage>,
        /// Receive SDK messages from the orchestrator (consumer side)
        sdk_rx: mpsc::Receiver<Result<SdkMessage>>,
        /// Receive JSON lines written by the orchestrator (captures stdin
        /// writes)
        stdin_rx: mpsc::Receiver<String>,
        /// Request tracker for outgoing control requests
        request_tracker: Arc<Mutex<RequestTracker>>,
        /// Cancellation token
        cancel: CancellationToken,
    }

    impl TestHarness {
        fn new(permission_handler: Option<Arc<dyn PermissionHandler>>) -> Self {
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
                Arc::new(stdin_writer),
                cancel.clone(),
                SessionCost::default(),
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
            match tokio::time::timeout(std::time::Duration::from_secs(2), self.sdk_rx.recv()).await
            {
                Ok(Some(Ok(msg))) => msg,
                Ok(Some(Err(e))) => panic!("Received error: {e}"),
                Ok(None) => panic!("Channel closed"),
                Err(_) => panic!("Timeout waiting for SDK message"),
            }
        }

        /// Read the next JSON line written to stdin (control response).
        async fn read_stdin_line(&mut self) -> serde_json::Value {
            match tokio::time::timeout(std::time::Duration::from_secs(2), self.stdin_rx.recv())
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
        let mut h = TestHarness::new(None);

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
        h.feed(RawStdoutMessage::Sdk(Box::new(sdk_msg))).await;

        let msg = h.recv().await;
        assert!(matches!(msg, SdkMessage::Assistant(_)));
    }

    #[tokio::test]
    async fn result_message_ends_stream() {
        let mut h = TestHarness::new(None);

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
        h.feed(RawStdoutMessage::Sdk(Box::new(sdk_msg))).await;

        let msg = h.recv().await;
        assert!(matches!(msg, SdkMessage::Result(_)));

        let next =
            tokio::time::timeout(std::time::Duration::from_millis(200), h.sdk_rx.recv()).await;
        assert!(next.is_ok()); // Returns Ok(None) — channel closed
        assert!(next.unwrap().is_none());
    }

    #[tokio::test]
    async fn keepalive_does_not_forward() {
        let mut h = TestHarness::new(None);

        h.feed(RawStdoutMessage::KeepAlive).await;

        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), h.sdk_rx.recv()).await;
        assert!(result.is_err()); // Timeout — no message forwarded
    }

    #[tokio::test]
    async fn unknown_message_does_not_forward() {
        let mut h = TestHarness::new(None);

        h.feed(RawStdoutMessage::Unknown(
            serde_json::json!({"type": "future_type"}),
        ))
        .await;

        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), h.sdk_rx.recv()).await;
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
        let mut h = TestHarness::new(Some(Arc::new(AllowAllHandler)));

        h.feed(make_can_use_tool_request("req-1", "Bash", "tu-1"))
            .await;

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
        let mut h = TestHarness::new(Some(Arc::new(handler)));

        h.feed(make_can_use_tool_request("req-2", "Write", "tu-2"))
            .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");
        assert_eq!(resp["response"]["behavior"], "deny");
        assert_eq!(resp["response"]["message"], "Not allowed");
        assert_eq!(resp["response"]["interrupt"], true);
    }

    #[tokio::test]
    async fn no_permission_handler_auto_allows() {
        let mut h = TestHarness::new(None);

        h.feed(make_can_use_tool_request("req-3", "Read", "tu-3"))
            .await;

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
        let mut h = TestHarness::new(Some(Arc::new(handler)));

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

        let _ = h.read_stdin_line().await;

        let guard = captured.lock().await;
        assert_eq!(guard.len(), 1);
        assert_eq!(guard[0].0, "Bash");
        assert_eq!(guard[0].1["command"], "ls -la");
    }

    #[tokio::test]
    async fn every_permission_request_is_answered() {
        let mut h = TestHarness::new(Some(Arc::new(AllowAllHandler)));

        h.feed(make_can_use_tool_request("req-a", "Read", "tu-a"))
            .await;
        h.feed(make_can_use_tool_request("req-b", "Write", "tu-b"))
            .await;

        // Decisions are made off the read loop and so may finish in either
        // order; the CLI pairs them by request id, not by arrival.
        let mut answered = vec![
            h.read_stdin_line().await["response"]["request_id"]
                .as_str()
                .unwrap()
                .to_string(),
            h.read_stdin_line().await["response"]["request_id"]
                .as_str()
                .unwrap()
                .to_string(),
        ];
        answered.sort();
        assert_eq!(answered, vec!["req-a", "req-b"]);
    }

    /// A handler that parks until it is released, like one waiting on a person.
    struct ParkingHandler {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl PermissionHandler for ParkingHandler {
        async fn can_use_tool(
            &self,
            _tool_name: &str,
            _input: &serde_json::Map<String, serde_json::Value>,
            _context: PermissionContext,
        ) -> PermissionResult {
            self.entered.notify_one();
            self.release.notified().await;
            PermissionResult::Allow {
                updated_input: None,
                updated_permissions: None,
            }
        }
    }

    /// Answered in line, it would block the only stdout reader and strand
    /// everything already written, making a run parked on a question look
    /// stopped.
    #[tokio::test]
    async fn a_parked_permission_decision_does_not_stall_the_message_stream() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let handler = ParkingHandler {
            entered: entered.clone(),
            release: release.clone(),
        };
        let mut h = TestHarness::new(Some(Arc::new(handler)));

        h.feed(make_can_use_tool_request(
            "req-park",
            "ExitPlanMode",
            "tu-park",
        ))
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(2), entered.notified())
            .await
            .expect("the handler is asked for a decision");

        // Written by the CLI after the request, and delivered while it stands.
        let assistant_json = serde_json::json!({
            "type": "assistant",
            "uuid": "msg-1",
            "session_id": "sess-1",
            "message": {
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "tu-park", "name": "ExitPlanMode",
                             "input": {}}]
            },
            "parent_tool_use_id": null
        });
        h.feed(RawStdoutMessage::Sdk(Box::new(
            SdkMessage::from_value(assistant_json).unwrap(),
        )))
        .await;
        assert!(matches!(h.recv().await, SdkMessage::Assistant(_)));

        // And the decision, once made, still answers its own request.
        release.notify_one();
        let response = h.read_stdin_line().await;
        assert_eq!(response["response"]["request_id"], "req-park");
        assert_eq!(response["response"]["response"]["behavior"], "allow");
    }

    // ============================================================
    // Hook Callback Tests
    // ============================================================

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
    async fn hook_callback_auto_responds_empty() {
        let mut h = TestHarness::new(None);

        h.feed(make_hook_callback_request(
            "req-h4",
            "cb-4",
            "PreToolUse",
            Some("tu-4"),
        ))
        .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"];
        assert_eq!(resp["subtype"], "success");
        let resp_value = resp["response"].as_object().unwrap();
        assert!(resp_value.is_empty());
    }

    // ============================================================
    // Control Response Routing Tests
    // ============================================================

    #[tokio::test]
    async fn control_response_resolves_pending_request() {
        let h = TestHarness::new(None);

        let (req_id, rx) = {
            let mut tracker = h.request_tracker.lock().await;
            tracker.register()
        };

        h.feed(RawStdoutMessage::ControlResponse(
            ControlResponseInner::Success {
                request_id: req_id.clone(),
                response: Some(serde_json::json!({"status": "ok"})),
            },
        ))
        .await;

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn control_error_response_resolves_as_error() {
        let h = TestHarness::new(None);

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
        let h = TestHarness::new(None);

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
        let mut h = TestHarness::new(None);

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
        h.feed(RawStdoutMessage::Sdk(Box::new(
            SdkMessage::from_value(json).unwrap(),
        )))
        .await;
        let _ = h.recv().await;

        h.cancel.cancel();

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), h.sdk_rx.recv()).await;
        // Either we get an error message or the channel closes
        match result {
            Ok(Some(Err(_))) => {} // Cancelled error
            Ok(None) => {}         // Channel closed
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
            Arc::new(stdin_writer),
            cancel.clone(),
            SessionCost::default(),
        );

        // Drop the feeder to signal EOF
        drop(msg_tx);

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), sdk_rx.recv()).await;
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
                    behavior: crate::api::permissions::PermissionBehavior::Allow,
                    destination: crate::api::permissions::PermissionUpdateDestination::Session,
                }]),
            }
        }
    }

    #[tokio::test]
    async fn permission_handler_allow_with_updated_input() {
        let mut h = TestHarness::new(Some(Arc::new(InputModifyingHandler)));

        h.feed(make_can_use_tool_request("req-mod", "Bash", "tu-mod"))
            .await;

        let response = h.read_stdin_line().await;
        let resp = &response["response"]["response"];
        assert_eq!(resp["behavior"], "allow");
        assert_eq!(resp["updatedInput"]["command"], "echo safe");
        assert!(resp["updatedPermissions"].is_array());
    }

    // ============================================================
    // MCP Message Tests
    // ============================================================

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
    async fn mcp_message_is_ignored() {
        let mut h = TestHarness::new(None);

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
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), h.stdin_rx.recv()).await;
        assert!(result.is_err()); // Timeout — no response written
    }
}
