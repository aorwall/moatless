//! `PermissionHandler`: that the CLI asks it before a tool runs, and that a
//! denial stops the tool.

use crate::harness::{TEST_MODEL, TEST_TIMEOUT, auth_env, collect_messages, init_tracing};
use async_trait::async_trait;
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::ContentBlock;
use claude_agent_sdk::PermissionHandler;
use claude_agent_sdk::PermissionResult;
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::api::permissions::PermissionContext;
use claude_agent_sdk::query;
use serde_json::Map;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::timeout;

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
    skip_unless_subprocess_e2e!();
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
    skip_unless_subprocess_e2e!();
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
