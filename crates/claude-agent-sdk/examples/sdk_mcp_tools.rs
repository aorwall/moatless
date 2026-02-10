//! Demonstrates in-process SDK MCP tools.
//!
//! This example registers custom tools that run in-process (no separate MCP
//! server process needed). The CLI sends `McpMessage` control requests for
//! these tools, and the SDK handles them directly.
//!
//! # Usage
//! ```bash
//! cargo run -p claude-agent-sdk --example sdk_mcp_tools
//! ```

use std::collections::HashMap;

use claude_agent_sdk::mcp::{SdkMcpServer, SdkMcpTool, ToolResult};
use claude_agent_sdk::types::messages::ResultMessage;
use claude_agent_sdk::{query, ClaudeOptions, ContentBlock, SdkMessage};

#[tokio::main]
async fn main() -> claude_agent_sdk::Result<()> {
    // Build an SDK MCP server with custom tools
    let calculator = SdkMcpServer::builder("calculator")
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
                .handler(|args| {
                    Box::pin(async move {
                        let a = args["a"].as_f64().unwrap_or(0.0);
                        let b = args["b"].as_f64().unwrap_or(0.0);
                        ToolResult::text(format!("{a} + {b} = {}", a + b))
                    })
                }),
        )
        .tool(
            SdkMcpTool::new("multiply", "Multiply two numbers together")
                .input_schema(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "a": {"type": "number", "description": "First number"},
                        "b": {"type": "number", "description": "Second number"}
                    },
                    "required": ["a", "b"]
                }))
                .handler(|args| {
                    Box::pin(async move {
                        let a = args["a"].as_f64().unwrap_or(0.0);
                        let b = args["b"].as_f64().unwrap_or(0.0);
                        ToolResult::text(format!("{a} * {b} = {}", a * b))
                    })
                }),
        )
        .build();

    // Register the SDK MCP server in options
    let mut sdk_mcp_servers = HashMap::new();
    sdk_mcp_servers.insert("calculator".to_string(), calculator);

    let options = ClaudeOptions {
        max_turns: Some(5),
        sdk_mcp_servers: Some(sdk_mcp_servers),
        ..Default::default()
    };

    let mut handle = query(
        "Use the calculator tools to compute (3 + 4) * 5. Show your work.",
        options,
    )
    .await?;

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
                println!("Done in {} turns. Cost: ${:.4}", result.num_turns, result.total_cost_usd);
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
