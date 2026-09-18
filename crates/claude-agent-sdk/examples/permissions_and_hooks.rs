// reason: examples print results to stdout/stderr for the reader.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Demonstrates custom permission handling and hooks.
//!
//! ```bash
//! cargo run -p claude-agent-sdk --example permissions_and_hooks
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use claude_agent_sdk::api::hooks::SyncHookOutput;
use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::api::permissions::PermissionContext;
use claude_agent_sdk::{
    ClaudeOptions, ContentBlock, HookHandler, HookOutput, PermissionHandler, PermissionMode,
    PermissionResult, SdkMessage, query,
};

/// A permission handler that only allows read-only tools.
struct ReadOnlyPermissions;

#[async_trait]
impl PermissionHandler for ReadOnlyPermissions {
    async fn can_use_tool(
        &self,
        tool_name: &str,
        _input: &serde_json::Map<String, serde_json::Value>,
        _context: PermissionContext,
    ) -> PermissionResult {
        match tool_name {
            // Allow read-only tools
            "Read" | "Glob" | "Grep" | "LS" | "WebSearch" | "WebFetch" => {
                println!("[Permission] Allowing tool: {tool_name}");
                PermissionResult::Allow {
                    updated_input: None,
                    updated_permissions: None,
                }
            }
            // Deny write tools
            _ => {
                println!("[Permission] Denying tool: {tool_name}");
                PermissionResult::Deny {
                    message: format!("Tool '{tool_name}' is not allowed in read-only mode"),
                    interrupt: None,
                }
            }
        }
    }
}

/// A hook handler that logs tool execution.
struct LoggingHookHandler;

#[async_trait]
impl HookHandler for LoggingHookHandler {
    async fn on_hook(&self, input: &serde_json::Value, tool_use_id: Option<&str>) -> HookOutput {
        let event = input
            .get("hookEventName")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let tool = input
            .get("toolName")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        println!(
            "[Hook] Event: {event}, Tool: {tool}, ID: {}",
            tool_use_id.unwrap_or("none")
        );

        // Continue execution with no modifications
        HookOutput::Sync(SyncHookOutput {
            should_continue: Some(true),
            ..Default::default()
        })
    }
}

#[tokio::main]
async fn main() -> claude_agent_sdk::Result<()> {
    let options = ClaudeOptions {
        max_turns: Some(3),
        permission_mode: Some(PermissionMode::Default),
        can_use_tool: Some(Arc::new(ReadOnlyPermissions)),
        hook_handler: Some(Arc::new(LoggingHookHandler)),
        ..Default::default()
    };

    let mut handle = query("List the files in the current directory.", options).await?;

    while let Some(msg) = handle.messages.recv().await {
        match msg? {
            SdkMessage::Assistant(assistant) => {
                for block in &assistant.message.content {
                    if let ContentBlock::Text { text } = block {
                        println!("Assistant: {text}");
                    }
                }
            }
            SdkMessage::Result(ResultMessage::Success(result)) => {
                println!(
                    "Done in {} turns. Cost: ${:.4}",
                    result.num_turns, result.total_cost_usd
                );
                break;
            }
            SdkMessage::Result(ResultMessage::Error(err)) => {
                eprintln!("Error: {:?}", err.errors);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
