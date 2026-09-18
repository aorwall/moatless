// reason: examples print results to stdout/stderr for the reader.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Simple one-shot query using the Claude Agent SDK.
//!
//! Needs the `claude` CLI on your PATH and an API key configured.
//!
//! ```bash
//! cargo run -p claude-agent-sdk --example simple_query
//! ```

use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::{ClaudeOptions, ContentBlock, SdkMessage, query};

#[tokio::main]
async fn main() -> claude_agent_sdk::Result<()> {
    // Configure options (defaults work for most cases)
    let options = ClaudeOptions {
        max_turns: Some(1),
        ..Default::default()
    };

    // Send a one-shot query
    let mut handle = query("What is 2 + 2? Reply with just the number.", options).await?;

    // Process messages from the CLI
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
                println!("Done. Cost: ${:.4}", result.total_cost_usd);
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
