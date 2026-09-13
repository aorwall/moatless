// reason: examples print results to stdout/stderr for the reader.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Multi-turn streaming conversation using `ClaudeClient`.
//!
//! Unlike one-shot `query()`, `ClaudeClient` keeps the session open across
//! exchanges.
//!
//! ```bash
//! cargo run -p claude-agent-sdk --example streaming_client
//! ```

use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::{ClaudeClient, ClaudeOptions, ContentBlock, SdkMessage};

#[tokio::main]
async fn main() -> claude_agent_sdk::Result<()> {
    let options = ClaudeOptions {
        max_turns: Some(3),
        ..Default::default()
    };

    let mut client = ClaudeClient::new(options).await?;

    // First turn
    println!("--- Turn 1 ---");
    client
        .send("Remember the number 42. Just confirm you've noted it.", "")
        .await?;

    let mut session_id = String::new();
    loop {
        match client.recv().await {
            Some(Ok(SdkMessage::System(init))) => {
                println!("Session: {}", init.session_id);
                session_id = init.session_id;
            }
            Some(Ok(SdkMessage::Assistant(assistant))) => {
                for block in &assistant.message.content {
                    if let ContentBlock::Text { text } = block {
                        println!("Assistant: {text}");
                    }
                }
            }
            Some(Ok(SdkMessage::Result(ResultMessage::Success(_)))) => break,
            Some(Ok(SdkMessage::Result(ResultMessage::Error(err)))) => {
                eprintln!("Error: {:?}", err.errors);
                client.close();
                return Ok(());
            }
            Some(Err(e)) => {
                eprintln!("Stream error: {e}");
                break;
            }
            None => break,
            _ => {}
        }
    }

    // Second turn (continues the same session)
    println!("\n--- Turn 2 ---");
    client
        .send("What number did I ask you to remember?", &session_id)
        .await?;

    loop {
        match client.recv().await {
            Some(Ok(SdkMessage::Assistant(assistant))) => {
                for block in &assistant.message.content {
                    if let ContentBlock::Text { text } = block {
                        println!("Assistant: {text}");
                    }
                }
            }
            Some(Ok(SdkMessage::Result(ResultMessage::Success(result)))) => {
                println!("Done. Cost: ${:.4}", result.total_cost_usd);
                break;
            }
            Some(Ok(SdkMessage::Result(ResultMessage::Error(err)))) => {
                eprintln!("Error: {:?}", err.errors);
                break;
            }
            Some(Err(e)) => {
                eprintln!("Stream error: {e}");
                break;
            }
            None => break,
            _ => {}
        }
    }

    client.close();
    Ok(())
}
