#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "examples print their results"
)]

//! Demonstrates custom permission handling.
//!
//! ```bash
//! cargo run -p claude-agent-sdk --example permissions
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::api::permissions::PermissionContext;
use claude_agent_sdk::{
    ClaudeOptions, ContentBlock, PermissionHandler, PermissionMode, PermissionResult, SdkMessage,
    query,
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
            "Read" | "Glob" | "Grep" | "LS" | "WebSearch" | "WebFetch" => {
                println!("[Permission] Allowing tool: {tool_name}");
                PermissionResult::Allow {
                    updated_input: None,
                    updated_permissions: None,
                }
            }
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

#[tokio::main]
async fn main() -> claude_agent_sdk::Result<()> {
    let options = ClaudeOptions {
        max_turns: Some(3),
        permission_mode: Some(PermissionMode::Default),
        can_use_tool: Some(Arc::new(ReadOnlyPermissions)),
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
