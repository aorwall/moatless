//! `HookHandler`: that a registered `PreToolUse` hook fires.

use crate::harness::{TEST_MODEL, TEST_TIMEOUT, auth_env, collect_messages, init_tracing};
use async_trait::async_trait;
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::HookDefinition;
use claude_agent_sdk::HookEvent;
use claude_agent_sdk::HookHandler;
use claude_agent_sdk::HookOutput;
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::SyncHookOutput;
use claude_agent_sdk::query;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::timeout;

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
    skip_unless_subprocess_e2e!();
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
