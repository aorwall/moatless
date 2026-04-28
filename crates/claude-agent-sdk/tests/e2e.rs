#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! End-to-end integration tests for claude-agent-sdk.
//!
//! These tests spawn the real `claude` CLI binary and make actual API calls.
//! They are gated behind the `CLAUDE_SDK_E2E=1` environment variable and
//! use `#[ignore]` so `cargo test` skips them by default.
//!
//! Run all E2E tests:
//!   CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk --ignored -- --nocapture
//!
//! Run a single test:
//!   CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk --ignored test_simple_query -- --nocapture
//!
//! Requirements:
//! - `claude` CLI installed and on PATH
//! - Valid ANTHROPIC_API_KEY in environment
//! - Each test costs ~$0.01-0.05 in API usage

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value};
use tokio::sync::Mutex;
use tokio::time::timeout;

use claude_agent_sdk::mcp::{SdkMcpServer, SdkMcpTool, ToolResult};
use claude_agent_sdk::types::agents::{AgentDefinition, AgentModel};
use claude_agent_sdk::types::messages::ResultMessage;
use claude_agent_sdk::types::permissions::PermissionContext;
use claude_agent_sdk::{
    query, ClaudeClient, ClaudeOptions, ContentBlock, HookDefinition, HookEvent, HookHandler,
    HookOutput, PermissionHandler, PermissionResult, SdkMessage, SyncHookOutput,
};

// ---------------------------------------------------------------------------
// Constants & Helpers
// ---------------------------------------------------------------------------

/// Test timeout — generous because CLI startup + API call can be slow.
const TEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Cheapest, fastest model for E2E tests.
const TEST_MODEL: &str = "claude-haiku-4-5-20251001";

/// Skip test unless CLAUDE_SDK_E2E=1 is set.
macro_rules! skip_unless_e2e {
    () => {
        if std::env::var("CLAUDE_SDK_E2E").as_deref() != Ok("1") {
            eprintln!("Skipping E2E test (set CLAUDE_SDK_E2E=1 to run)");
            return;
        }
    };
}

/// Initialize tracing for test output (idempotent).
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("claude_agent_sdk=debug".parse().unwrap()),
        )
        .with_test_writer()
        .try_init();
}

/// Collect auth-related env vars to forward explicitly to the CLI subprocess.
fn auth_env() -> Option<HashMap<String, String>> {
    let mut env = HashMap::new();
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        env.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token);
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env.insert("ANTHROPIC_API_KEY".to_string(), key);
    }
    if env.is_empty() {
        None
    } else {
        Some(env)
    }
}

/// Default options for E2E tests: haiku model, 1 turn, skip permissions.
fn e2e_options() -> ClaudeOptions {
    ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(1),
        allow_dangerously_skip_permissions: Some(true),
        env: auth_env(),
        ..Default::default()
    }
}

/// Collect all messages from a query handle until Result or channel close.
async fn collect_messages(
    rx: &mut tokio::sync::mpsc::Receiver<claude_agent_sdk::Result<SdkMessage>>,
) -> Vec<SdkMessage> {
    let mut messages = Vec::new();
    while let Some(msg) = rx.recv().await {
        match msg {
            Ok(msg) => {
                let is_result = matches!(&msg, SdkMessage::Result(_));
                messages.push(msg);
                if is_result {
                    break;
                }
            }
            Err(e) => {
                eprintln!("Error receiving message: {e}");
                break;
            }
        }
    }
    messages
}

/// Extract all text from assistant messages.
fn extract_assistant_text(messages: &[SdkMessage]) -> String {
    let mut text = String::new();
    for msg in messages {
        if let SdkMessage::Assistant(assistant) = msg {
            for block in &assistant.message.content {
                if let ContentBlock::Text { text: t } = block {
                    text.push_str(t);
                }
            }
        }
    }
    text
}

/// Extract the result text from a successful Result message.
fn extract_result_text(messages: &[SdkMessage]) -> Option<String> {
    messages.iter().find_map(|m| {
        if let SdkMessage::Result(ResultMessage::Success(s)) = m {
            Some(s.result.clone())
        } else {
            None
        }
    })
}

// ===========================================================================
// Test 1: Simple query — verify basic message flow and result
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_simple_query() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query("What is 2+2? Reply with only the number.", e2e_options())
            .await
            .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Should have at least: System init, Assistant, Result
        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::System(_))),
            "Expected System init message"
        );
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, SdkMessage::Assistant(_))),
            "Expected Assistant message"
        );
        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected Result message"
        );

        // Check result contains "4"
        let result_text = extract_result_text(&messages).expect("Expected successful result");
        assert!(
            result_text.contains('4'),
            "Expected result to contain '4', got: {result_text}"
        );

        // Check cost and turns
        if let Some(SdkMessage::Result(ResultMessage::Success(success))) =
            messages.iter().find(|m| matches!(m, SdkMessage::Result(_)))
        {
            assert!(
                success.total_cost_usd > 0.0,
                "Expected non-zero cost, got: {}",
                success.total_cost_usd
            );
            assert!(
                success.num_turns >= 1,
                "Expected at least 1 turn, got: {}",
                success.num_turns
            );
        }
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 2: Result contains session ID matching System init
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_result_contains_session_id() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query("Say hello.", e2e_options())
            .await
            .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        let system_session_id = messages.iter().find_map(|m| {
            if let SdkMessage::System(init) = m {
                Some(init.session_id.clone())
            } else {
                None
            }
        });

        let result_session_id = messages.iter().find_map(|m| match m {
            SdkMessage::Result(ResultMessage::Success(s)) => Some(s.session_id.clone()),
            SdkMessage::Result(ResultMessage::Error(e)) => Some(e.session_id.clone()),
            _ => None,
        });

        let sys_id = system_session_id.expect("System init should have session_id");
        let res_id = result_session_id.expect("Result should have session_id");

        assert!(!sys_id.is_empty(), "System session_id should be non-empty");
        assert!(!res_id.is_empty(), "Result session_id should be non-empty");
        assert_eq!(sys_id, res_id, "Session IDs should match");
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 3: System init contains model and tools
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_system_init_has_model_and_tools() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query("Say hi.", e2e_options())
            .await
            .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        let init = messages
            .iter()
            .find_map(|m| {
                if let SdkMessage::System(init) = m {
                    Some(init)
                } else {
                    None
                }
            })
            .expect("Expected System init message");

        // Model should contain our requested model (haiku)
        assert!(
            init.model.contains("haiku"),
            "Expected model to contain 'haiku', got: {}",
            init.model
        );

        // Tools list should be non-empty
        assert!(
            !init.tools.is_empty(),
            "Expected non-empty tools list, got: {:?}",
            init.tools
        );

        eprintln!("Model: {}", init.model);
        eprintln!("Tools ({}): {:?}", init.tools.len(), init.tools);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 4: Permission handler is invoked (allow-all, capture calls)
// ===========================================================================

/// A permission handler that allows all tools and records invocations.
struct CapturingPermissionHandler {
    invocations: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PermissionHandler for CapturingPermissionHandler {
    async fn can_use_tool(
        &self,
        tool_name: &str,
        _input: &Map<String, Value>,
        _context: PermissionContext,
    ) -> PermissionResult {
        self.invocations.lock().await.push(tool_name.to_string());
        PermissionResult::Allow {
            updated_input: None,
            updated_permissions: None,
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_permission_handler_is_invoked() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        // Use a temp directory to avoid project-level settings auto-allowing tools.
        let tmp = std::env::temp_dir().join("claude-sdk-e2e-perm");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        std::fs::write(tmp.join("hello.txt"), "Hello from E2E test").expect("write test file");

        let invocations = Arc::new(Mutex::new(Vec::new()));

        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(3),
            cwd: Some(tmp.to_string_lossy().to_string()),
            can_use_tool: Some(Arc::new(CapturingPermissionHandler {
                invocations: invocations.clone(),
            })),
            env: auth_env(),
            ..Default::default()
        };

        let mut handle = query(
            "Write the text 'Hello from test' to a new file called output.txt in the current directory. Use the Write tool.",
            options,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Print all messages for debugging
        for msg in &messages {
            match msg {
                SdkMessage::System(_) => eprintln!("  [System]"),
                SdkMessage::Assistant(a) => {
                    let text: String = a.message.content.iter().filter_map(|b| {
                        if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None }
                    }).collect();
                    eprintln!("  [Assistant] {}", &text[..text.len().min(120)]);
                }
                SdkMessage::Result(ResultMessage::Success(s)) => {
                    eprintln!("  [Result:Success] turns={} cost=${:.4}", s.num_turns, s.total_cost_usd);
                }
                SdkMessage::Result(ResultMessage::Error(e)) => {
                    eprintln!("  [Result:Error] {:?}", e.errors);
                }
                other => eprintln!("  [{:?}]", std::mem::discriminant(other)),
            }
        }

        assert!(
            messages
                .iter()
                .any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected Result message"
        );

        let captured = invocations.lock().await;
        assert!(
            !captured.is_empty(),
            "Expected permission handler to be invoked at least once, but it was never called"
        );
        eprintln!("Permission handler invoked for tools: {:?}", *captured);

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 5: Permission handler deny blocks tool use
// ===========================================================================

/// A permission handler that denies all tool use.
struct DenyAllPermissionHandler;

#[async_trait]
impl PermissionHandler for DenyAllPermissionHandler {
    async fn can_use_tool(
        &self,
        _tool_name: &str,
        _input: &Map<String, Value>,
        _context: PermissionContext,
    ) -> PermissionResult {
        PermissionResult::Deny {
            message: "All tools denied by test handler".to_string(),
            interrupt: None,
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_permission_handler_deny_blocks_tool() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        // Use a temp directory to avoid project-level settings auto-allowing tools.
        let tmp = std::env::temp_dir().join("claude-sdk-e2e-deny");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        std::fs::write(tmp.join("hello.txt"), "Secret content").expect("write test file");

        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(2),
            cwd: Some(tmp.to_string_lossy().to_string()),
            can_use_tool: Some(Arc::new(DenyAllPermissionHandler)),
            env: auth_env(),
            ..Default::default()
        };

        let mut handle = query(
            "Write the text 'test content' to a file called output.txt in the current directory.",
            options,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        let result_msg = messages
            .iter()
            .find(|m| matches!(m, SdkMessage::Result(_)))
            .expect("Expected Result message");

        match result_msg {
            SdkMessage::Result(ResultMessage::Success(success)) => {
                // Either permission_denials is non-empty, or the result text
                // mentions the denial in some form
                let has_denials = !success.permission_denials.is_empty();
                let text_mentions_denial = {
                    let lower = success.result.to_lowercase();
                    lower.contains("denied")
                        || lower.contains("permission")
                        || lower.contains("blocked")
                        || lower.contains("unable")
                        || lower.contains("cannot")
                        || lower.contains("not allowed")
                };
                assert!(
                    has_denials || text_mentions_denial,
                    "Expected permission denials or denial mention in result. \
                     Denials: {:?}, Result: {}",
                    success.permission_denials,
                    success.result
                );
            }
            SdkMessage::Result(ResultMessage::Error(error)) => {
                // Error result is acceptable — the tool was blocked
                eprintln!("Got error result (expected): {:?}", error.errors);
            }
            _ => unreachable!(),
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 6: Hook handler is invoked for PreToolUse
// ===========================================================================

/// A hook handler that records all invocations.
struct CapturingHookHandler {
    invocations: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl HookHandler for CapturingHookHandler {
    async fn on_hook(&self, input: &Value, _tool_use_id: Option<&str>) -> HookOutput {
        self.invocations.lock().await.push(input.clone());
        // Allow everything — just observe
        HookOutput::Sync(SyncHookOutput {
            should_continue: Some(true),
            ..Default::default()
        })
    }
}

#[tokio::test]
#[ignore]
async fn test_hook_handler_is_invoked() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let invocations = Arc::new(Mutex::new(Vec::new()));

        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(3),
            allow_dangerously_skip_permissions: Some(true),
            hooks: Some(vec![HookDefinition {
                event: HookEvent::PreToolUse,
                matcher: None, // match all tools
                timeout: Some(10),
            }]),
            hook_handler: Some(Arc::new(CapturingHookHandler {
                invocations: invocations.clone(),
            })),
            env: auth_env(),
            ..Default::default()
        };

        let mut handle = query(
            "Read the file ./Cargo.toml and tell me the crate name.",
            options,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected Result message"
        );

        let captured = invocations.lock().await;
        assert!(
            !captured.is_empty(),
            "Expected hook handler to be invoked at least once"
        );
        eprintln!("Hook handler invoked {} time(s)", captured.len());
        for (i, input) in captured.iter().enumerate() {
            eprintln!("  Hook invocation {i}: {input}");
        }
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 7: SDK MCP tool is called
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_sdk_mcp_tool_is_called() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let tool_was_called = Arc::new(AtomicBool::new(false));
        let tool_flag = tool_was_called.clone();

        let server = SdkMcpServer::builder("test-calc")
            .tool(
                SdkMcpTool::new("add", "Add two numbers together")
                    .input_schema(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "a": {"type": "number", "description": "First number"},
                            "b": {"type": "number", "description": "Second number"}
                        },
                        "required": ["a", "b"]
                    }))
                    .handler(move |args| {
                        let flag = tool_flag.clone();
                        Box::pin(async move {
                            flag.store(true, Ordering::SeqCst);
                            let a = args["a"].as_f64().unwrap_or(0.0);
                            let b = args["b"].as_f64().unwrap_or(0.0);
                            ToolResult::text(format!("{}", a + b))
                        })
                    }),
            )
            .build();

        let mut sdk_mcp_servers = HashMap::new();
        sdk_mcp_servers.insert("test-calc".to_string(), server);

        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(3),
            allow_dangerously_skip_permissions: Some(true),
            allowed_tools: Some(vec!["mcp__test-calc__add".to_string()]),
            sdk_mcp_servers: Some(sdk_mcp_servers),
            env: auth_env(),
            ..Default::default()
        };

        let mut handle = query(
            "Use the add tool from the test-calc MCP server to add 3 and 4. \
             Report only the numeric result.",
            options,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Debug: print all messages
        for msg in &messages {
            match msg {
                SdkMessage::System(_) => eprintln!("  [System]"),
                SdkMessage::Assistant(a) => {
                    let text: String = a
                        .message
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();
                    eprintln!("  [Assistant] {}", &text[..text.len().min(200)]);
                }
                SdkMessage::Result(ResultMessage::Success(s)) => {
                    eprintln!(
                        "  [Result:Success] turns={} cost=${:.4} text={}",
                        s.num_turns,
                        s.total_cost_usd,
                        &s.result[..s.result.len().min(200)]
                    );
                }
                SdkMessage::Result(ResultMessage::Error(e)) => {
                    eprintln!("  [Result:Error] {:?}", e.errors);
                }
                other => eprintln!("  [{:?}]", std::mem::discriminant(other)),
            }
        }

        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected Result message"
        );

        // Verify the tool was actually invoked
        assert!(
            tool_was_called.load(Ordering::SeqCst),
            "Expected the add tool to be called"
        );

        // Verify the result mentions 7
        let result_text = extract_result_text(&messages);
        let assistant_text = extract_assistant_text(&messages);
        let combined = format!(
            "{} {}",
            assistant_text,
            result_text.as_deref().unwrap_or("")
        );
        assert!(
            combined.contains('7'),
            "Expected response to mention '7', got: {combined}"
        );

        eprintln!("Assistant said: {assistant_text}");
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 8: Streaming client multi-turn conversation
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_streaming_client_multi_turn() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(Duration::from_secs(90), async {
        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(1),
            allow_dangerously_skip_permissions: Some(true),
            env: auth_env(),
            ..Default::default()
        };

        let mut client = ClaudeClient::new(options)
            .await
            .expect("Failed to create ClaudeClient");

        // Turn 1: Ask to remember a number
        client
            .send(
                "Remember the number 42. Just confirm you've noted it. Be very brief.",
                "",
            )
            .await
            .expect("Failed to send first message");

        let mut session_id = String::new();
        let mut turn1_messages = Vec::new();
        while let Some(msg) = client.recv().await {
            match msg {
                Ok(msg) => {
                    if let SdkMessage::System(ref init) = msg {
                        session_id = init.session_id.clone();
                    }
                    let is_result = matches!(&msg, SdkMessage::Result(_));
                    turn1_messages.push(msg);
                    if is_result {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Turn 1 error: {e}");
                    break;
                }
            }
        }

        assert!(
            turn1_messages
                .iter()
                .any(|m| matches!(m, SdkMessage::Result(_))),
            "Turn 1 should produce a Result"
        );
        assert!(
            !session_id.is_empty(),
            "Should have received session_id from System init"
        );
        eprintln!(
            "Turn 1 result: {}",
            extract_result_text(&turn1_messages).unwrap_or_default()
        );

        // Turn 2: Ask what number was remembered
        // NOTE: The current orchestrator exits after the first Result message,
        // which means multi-turn via ClaudeClient may not work yet. This test
        // verifies the current behavior and will pass once multi-turn is fixed.
        match client
            .send(
                "What number did I ask you to remember? Reply with just the number.",
                &session_id,
            )
            .await
        {
            Ok(()) => {}
            Err(e) => {
                eprintln!(
                    "WARNING: Failed to send turn 2 message \
                     (expected if orchestrator exits on Result): {e}"
                );
                client.close();
                return;
            }
        }

        let mut turn2_messages = Vec::new();
        while let Some(msg) = client.recv().await {
            match msg {
                Ok(msg) => {
                    let is_result = matches!(&msg, SdkMessage::Result(_));
                    turn2_messages.push(msg);
                    if is_result {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Turn 2 error: {e}");
                    break;
                }
            }
        }

        if turn2_messages.is_empty() {
            eprintln!(
                "WARNING: Turn 2 produced no messages. Multi-turn is not yet \
                 supported — the orchestrator exits after delivering a Result message."
            );
            client.close();
            return;
        }

        // Verify the response mentions 42
        let turn2_text = extract_assistant_text(&turn2_messages);
        let turn2_result = extract_result_text(&turn2_messages).unwrap_or_default();
        let combined = format!("{turn2_text} {turn2_result}");
        assert!(
            combined.contains("42"),
            "Expected turn 2 response to mention '42', got: {combined}"
        );

        client.close();
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 9: Session resume with --resume recalls conversation context
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_session_resume_recalls_context() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(Duration::from_secs(90), async {
        // Use a temp dir so session files are isolated
        let tmp = std::env::temp_dir().join("claude-sdk-e2e-resume");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let base_options = || ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(1),
            allow_dangerously_skip_permissions: Some(true),
            cwd: Some(tmp.to_string_lossy().to_string()),
            env: auth_env(),
            ..Default::default()
        };

        // Turn 1: Ask the model to write code containing a distinctive string
        let mut handle = query(
            "Write a Rust function called `get_answer()` that returns the &str \"blue elephant\". \
             Just show the function, nothing else.",
            base_options(),
        )
        .await
        .expect("Failed to start turn 1");

        let messages = collect_messages(&mut handle.messages).await;

        let session_id = messages
            .iter()
            .find_map(|m| {
                if let SdkMessage::System(init) = m {
                    Some(init.session_id.clone())
                } else {
                    None
                }
            })
            .expect("Turn 1 should have System init with session_id");

        let turn1_result = extract_result_text(&messages).expect("Turn 1 should succeed");
        eprintln!("Turn 1 session_id: {session_id}");
        eprintln!("Turn 1 result: {turn1_result}");

        let turn1_lower = turn1_result.to_lowercase();
        assert!(
            turn1_lower.contains("blue elephant"),
            "Turn 1 should contain 'blue elephant', got: {turn1_result}"
        );

        // Turn 2: Resume the session and ask about the previous code
        let resume_options = ClaudeOptions {
            resume: Some(session_id.clone()),
            ..base_options()
        };

        let mut handle = query(
            "What string does the `get_answer()` function return? Reply with just the string value.",
            resume_options,
        )
        .await
        .expect("Failed to start turn 2");

        let messages = collect_messages(&mut handle.messages).await;

        let turn2_session_id = messages.iter().find_map(|m| {
            if let SdkMessage::System(init) = m {
                Some(init.session_id.clone())
            } else {
                None
            }
        });

        let turn2_result = extract_result_text(&messages).expect("Turn 2 should succeed");
        eprintln!("Turn 2 session_id: {turn2_session_id:?}");
        eprintln!("Turn 2 result: {turn2_result}");

        // Session ID should match (same session resumed)
        if let Some(ref t2_id) = turn2_session_id {
            assert_eq!(
                &session_id, t2_id,
                "Resumed session should have the same session_id"
            );
        }

        // Model should recall the string from turn 1
        let turn2_lower = turn2_result.to_lowercase();
        assert!(
            turn2_lower.contains("blue elephant"),
            "Turn 2 should recall 'blue elephant' from turn 1, got: {turn2_result}"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 10: Session continue with --continue recalls conversation context
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_session_continue_recalls_context() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(Duration::from_secs(90), async {
        // Use a temp dir so session files are isolated
        let tmp = std::env::temp_dir().join("claude-sdk-e2e-continue");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let base_options = || ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(1),
            allow_dangerously_skip_permissions: Some(true),
            cwd: Some(tmp.to_string_lossy().to_string()),
            env: auth_env(),
            ..Default::default()
        };

        // Turn 1: Ask the model to write code containing a distinctive string
        let mut handle = query(
            "Write a Rust function called `get_answer()` that returns the &str \"purple giraffe\". \
             Just show the function, nothing else.",
            base_options(),
        )
        .await
        .expect("Failed to start turn 1");

        let messages = collect_messages(&mut handle.messages).await;

        let turn1_result = extract_result_text(&messages).expect("Turn 1 should succeed");
        eprintln!("Turn 1 result: {turn1_result}");

        let turn1_lower = turn1_result.to_lowercase();
        assert!(
            turn1_lower.contains("purple giraffe"),
            "Turn 1 should contain 'purple giraffe', got: {turn1_result}"
        );

        // Turn 2: Continue the most recent session in this directory
        let continue_options = ClaudeOptions {
            continue_conversation: Some(true),
            ..base_options()
        };

        let mut handle = query(
            "What string does the `get_answer()` function return? Reply with just the string value.",
            continue_options,
        )
        .await
        .expect("Failed to start turn 2");

        let messages = collect_messages(&mut handle.messages).await;

        let turn2_result = extract_result_text(&messages).expect("Turn 2 should succeed");
        eprintln!("Turn 2 result: {turn2_result}");

        // Model should recall the string from turn 1
        let turn2_lower = turn2_result.to_lowercase();
        assert!(
            turn2_lower.contains("purple giraffe"),
            "Turn 2 should recall 'purple giraffe' from turn 1, got: {turn2_result}"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 11: Sub-agent round trip — verify enriched fields on messages
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_subagent_fields_captured() {
    skip_unless_e2e!();
    init_tracing();

    let mut agents = HashMap::new();
    agents.insert(
        "helper".to_string(),
        AgentDefinition {
            description: "A helper agent for simple tasks".to_string(),
            prompt: "You are a helpful assistant. Answer concisely.".to_string(),
            tools: None,
            disallowed_tools: None,
            model: Some(AgentModel::Haiku),
            mcp_servers: None,
            skills: None,
            max_turns: Some(1),
        },
    );

    let opts = ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(3),
        allow_dangerously_skip_permissions: Some(true),
        agents: Some(agents),
        env: auth_env(),
        ..Default::default()
    };

    let result = timeout(Duration::from_secs(90), async {
        let mut handle = query(
            "Use the Agent tool to ask the 'helper' agent: What is 1+1? Reply with only the number. Then tell me what the agent said.",
            opts,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Verify at least one assistant message has ApiMessage enriched fields.
        // Not every assistant message has stop_reason (thinking-only partials don't),
        // so we collect all and check that at least one has model + id populated.
        let assistants: Vec<_> = messages
            .iter()
            .filter_map(|m| {
                if let SdkMessage::Assistant(a) = m {
                    Some(a)
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !assistants.is_empty(),
            "Expected at least one assistant message"
        );

        let has_model = assistants.iter().any(|a| a.message.model.is_some());
        let has_id = assistants.iter().any(|a| a.message.id.is_some());
        let has_stop_reason = assistants.iter().any(|a| a.message.stop_reason.is_some());

        assert!(has_model, "Expected at least one assistant message with model");
        assert!(has_id, "Expected at least one assistant message with id");
        // stop_reason may only appear on the final assistant message of each turn
        println!("has_stop_reason: {has_stop_reason}");

        // Verify user messages (tool results) exist with toolUseResult
        let user_msgs: Vec<_> = messages
            .iter()
            .filter_map(|m| {
                if let SdkMessage::User(u) = m {
                    Some(u)
                } else {
                    None
                }
            })
            .collect();

        // Print for debugging
        for u in &user_msgs {
            if u.tool_use_result.is_some() {
                println!(
                    "User message has toolUseResult: {}",
                    serde_json::to_string_pretty(&u.tool_use_result).unwrap()
                );
            }
        }

        // At least one user message should have tool_use_result
        // (the Agent tool returns a result object)
        let has_agent_result = user_msgs.iter().any(|u| u.tool_use_result.is_some());
        println!("Has agent tool_use_result: {has_agent_result}");

        // Print all message types for debugging
        for (i, msg) in messages.iter().enumerate() {
            match msg {
                SdkMessage::System(_) => println!("[{i}] System"),
                SdkMessage::Assistant(a) => println!(
                    "[{i}] Assistant (model={:?}, id={:?}, stop_reason={:?})",
                    a.message.model, a.message.id, a.message.stop_reason
                ),
                SdkMessage::User(u) => println!(
                    "[{i}] User (is_meta={:?}, tool_use_result={})",
                    u.is_meta,
                    u.tool_use_result.is_some()
                ),
                SdkMessage::UserReplay(_) => println!("[{i}] UserReplay"),
                SdkMessage::Result(_) => println!("[{i}] Result"),
                SdkMessage::TaskNotification(t) => println!(
                    "[{i}] TaskNotification (status={}, task_id={})",
                    t.status, t.task_id
                ),
                _ => println!("[{i}] Other"),
            }
        }

        messages
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Diagnostic: Empty result.result when turn ends after a tool_use
// ===========================================================================

/// Reproduces the bug observed in production: an assistant message that
/// contains BOTH text and a tool_use, where the agent has nothing more to
/// say after the tool returns. We want to know what the CLI sends in the
/// final `result` message — is `result.result` populated with the assistant's
/// text, or is it empty?
///
/// This test does NOT assert that the result is empty (the CLI may change).
/// It prints everything so we can see the real behavior, plus asserts a
/// MINIMAL invariant: a Result message arrives. Look at the eprintln! output
/// (`-- --nocapture`) for the actual values.
#[tokio::test]
#[ignore]
async fn test_diagnose_empty_result_after_trailing_tool_use() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(Duration::from_secs(90), async {
        // Use a temp directory so the Bash tool can run without leaking.
        let tmp = std::env::temp_dir().join("claude-sdk-e2e-empty-result");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            // Generous turns so the model can: turn1 = text+tool_use, then
            // (optionally) turn2 = no further text. The bug appears if the
            // CLI emits result.result="" even when an assistant text was
            // produced earlier in the same turn.
            max_turns: Some(3),
            allow_dangerously_skip_permissions: Some(true),
            allowed_tools: Some(vec!["Bash".to_string()]),
            cwd: Some(tmp.to_string_lossy().to_string()),
            env: auth_env(),
            ..Default::default()
        };

        // Prompt engineered to mimic the production case:
        //   - Give a textual answer (the "what model are you?" reply)
        //   - Then run a benign Bash command (the "moat set-name" call)
        //   - Explicitly tell the model to stop talking after the tool returns
        let prompt = "Step 1: Answer the question \"what is 2+2?\" with a one-sentence \
                      reply (e.g. \"2+2 equals 4.\"). \
                      Step 2: Run the shell command `echo done` using the Bash tool. \
                      Step 3: After the Bash tool returns, output absolutely no \
                      additional text. Do not summarize, confirm, or comment. Just stop.";

        let mut handle = query(prompt, options).await.expect("query failed");
        let messages = collect_messages(&mut handle.messages).await;

        // ── Diagnostic dump: print every message we received ──
        eprintln!("\n=== EMPTY-RESULT DIAGNOSTIC ===");
        eprintln!("Total messages: {}", messages.len());
        for (i, msg) in messages.iter().enumerate() {
            match msg {
                SdkMessage::System(init) => {
                    eprintln!("[{i}] System init: model={} session={}", init.model, init.session_id);
                }
                SdkMessage::Assistant(a) => {
                    eprintln!(
                        "[{i}] Assistant id={:?} model={:?} stop_reason={:?} blocks={}",
                        a.message.id, a.message.model, a.message.stop_reason,
                        a.message.content.len()
                    );
                    for (b, block) in a.message.content.iter().enumerate() {
                        match block {
                            ContentBlock::Text { text } => {
                                let preview = &text[..text.len().min(120)];
                                eprintln!("      block[{b}] text: {preview:?}");
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                eprintln!(
                                    "      block[{b}] tool_use id={id} name={name} input={}",
                                    serde_json::to_string(input).unwrap_or_default()
                                );
                            }
                            other => eprintln!("      block[{b}] {:?}", std::mem::discriminant(other)),
                        }
                    }
                }
                SdkMessage::User(u) => {
                    eprintln!(
                        "[{i}] User (tool result) blocks={} tool_use_result_present={}",
                        u.message.content.len(), u.tool_use_result.is_some()
                    );
                    for (b, block) in u.message.content.iter().enumerate() {
                        match block {
                            ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                                let summary = match content {
                                    Some(serde_json::Value::String(s)) => s.clone(),
                                    Some(other) => other.to_string(),
                                    None => "<no content>".to_string(),
                                };
                                let preview = &summary[..summary.len().min(120)];
                                eprintln!(
                                    "      block[{b}] tool_result id={tool_use_id} is_error={is_error:?} content={preview:?}"
                                );
                            }
                            ContentBlock::Text { text } => {
                                let preview = &text[..text.len().min(120)];
                                eprintln!("      block[{b}] text: {preview:?}");
                            }
                            other => eprintln!("      block[{b}] {:?}", std::mem::discriminant(other)),
                        }
                    }
                }
                SdkMessage::Result(ResultMessage::Success(s)) => {
                    eprintln!(
                        "[{i}] Result:Success turns={} duration={}ms cost=${:.4} is_error={}",
                        s.num_turns, s.duration_ms, s.total_cost_usd, s.is_error
                    );
                    eprintln!("      result.result.len() = {}", s.result.len());
                    eprintln!("      result.result = {:?}", s.result);
                    eprintln!("      model_usage keys: {:?}", s.model_usage.keys().collect::<Vec<_>>());
                }
                SdkMessage::Result(ResultMessage::Error(e)) => {
                    eprintln!("[{i}] Result:Error subtype={} errors={:?}", e.subtype, e.errors);
                }
                other => eprintln!("[{i}] {:?}", std::mem::discriminant(other)),
            }
        }

        // ── Cross-check: did assistant emit text? did it emit a tool_use? ──
        let assistant_text = extract_assistant_text(&messages);
        let assistant_emitted_tool_use = messages.iter().any(|m| {
            if let SdkMessage::Assistant(a) = m {
                a.message.content.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }))
            } else {
                false
            }
        });
        let result_text = extract_result_text(&messages).unwrap_or_default();

        eprintln!("--- summary ---");
        eprintln!("assistant emitted text? {} (len={})", !assistant_text.is_empty(), assistant_text.len());
        eprintln!("assistant emitted tool_use? {assistant_emitted_tool_use}");
        eprintln!("result.result empty? {} (len={})", result_text.is_empty(), result_text.len());
        eprintln!("=== END DIAGNOSTIC ===\n");

        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected a Result message"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Diagnostic: Cost field visibility
// ===========================================================================

/// Diagnostic test to troubleshoot missing cost data.
/// Prints detailed cost/usage information from the CLI result message.
#[tokio::test]
#[ignore]
async fn test_diagnose_cost_fields() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query("Say hi.", e2e_options())
            .await
            .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Check if we received a Result at all
        let has_result = messages.iter().any(|m| matches!(m, SdkMessage::Result(_)));
        eprintln!("\n=== COST DIAGNOSTIC ===");
        eprintln!("Total messages received: {}", messages.len());
        eprintln!("Has Result message: {has_result}");

        if !has_result {
            eprintln!("WARNING: No Result message received!");
            eprintln!("This likely means ResultSuccess deserialization failed.");
            eprintln!("Check warn-level logs above for 'Failed to deserialize SDK message'.");
            panic!("No Result message received — deserialization likely failed");
        }

        match messages.iter().find(|m| matches!(m, SdkMessage::Result(_))) {
            Some(SdkMessage::Result(ResultMessage::Success(s))) => {
                eprintln!("Result type: Success");
                eprintln!("  total_cost_usd: {}", s.total_cost_usd);
                eprintln!("  num_turns: {}", s.num_turns);
                eprintln!("  duration_ms: {}", s.duration_ms);
                eprintln!("  is_error: {}", s.is_error);
                eprintln!("  usage.input_tokens: {}", s.usage.input_tokens);
                eprintln!("  usage.output_tokens: {}", s.usage.output_tokens);
                eprintln!(
                    "  usage.cache_read_input_tokens: {:?}",
                    s.usage.cache_read_input_tokens
                );
                eprintln!(
                    "  usage.cache_creation_input_tokens: {:?}",
                    s.usage.cache_creation_input_tokens
                );
                eprintln!("  model_usage entries: {}", s.model_usage.len());
                for (model, mu) in &s.model_usage {
                    eprintln!(
                        "    {model}: cost_usd={} in={} out={}",
                        mu.cost_usd, mu.input_tokens, mu.output_tokens
                    );
                }

                // Serialize back to JSON to see what the payload looks like downstream
                let payload = serde_json::to_value(s).unwrap();
                eprintln!(
                    "  payload.total_cost_usd: {:?}",
                    payload.get("total_cost_usd")
                );

                assert!(
                    s.total_cost_usd > 0.0,
                    "Expected non-zero cost, got: {}",
                    s.total_cost_usd
                );
            }
            Some(SdkMessage::Result(ResultMessage::Error(e))) => {
                eprintln!("Result type: Error");
                eprintln!("  subtype: {}", e.subtype);
                eprintln!("  total_cost_usd: {}", e.total_cost_usd);
                eprintln!("  errors: {:?}", e.errors);
                panic!("Got ResultError instead of ResultSuccess");
            }
            _ => unreachable!(),
        }

        eprintln!("=== END DIAGNOSTIC ===\n");
    })
    .await;

    result.expect("Test timed out");
}
