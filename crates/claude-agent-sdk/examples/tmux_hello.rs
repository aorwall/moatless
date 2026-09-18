// reason: examples print results to stdout/stderr for the reader.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Tmux-mode one-shot query: drives the real interactive Claude TUI in a
//! detached tmux session and reads its JSONL transcript as `SdkMessage`s.
//!
//! Needs `tmux` and `claude` on your PATH. Attach with `tmux attach -t
//! claude-sdk-<8 hex>` (`tmux ls` names it) to watch the TUI.
//!
//! ```bash
//! cargo run -p claude-agent-sdk --example tmux_hello
//! ```

use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::{ClaudeOptions, ContentBlock, SdkMessage, query_tmux};

#[tokio::main]
async fn main() -> claude_agent_sdk::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "claude_agent_sdk=debug,info".into()),
        )
        .init();

    let options = ClaudeOptions {
        max_turns: Some(1),
        ..Default::default()
    };

    let mut handle = query_tmux("Say hello in exactly three words.", options).await?;

    while let Some(msg) = handle.messages.recv().await {
        match msg? {
            SdkMessage::System(init) => {
                println!(
                    "System init: session={} model={}",
                    init.session_id, init.model
                );
            }
            SdkMessage::Assistant(assistant) => {
                for block in &assistant.message.content {
                    if let ContentBlock::Text { text } = block {
                        println!("Assistant: {text}");
                    }
                }
            }
            SdkMessage::Result(ResultMessage::Success(result)) => {
                println!(
                    "Done. Turn {} took {}ms (input={} output={})",
                    result.num_turns,
                    result.duration_ms,
                    result.usage.input_tokens,
                    result.usage.output_tokens
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
