// Allow unwrap/expect/print in test code — production code still enforces the deny.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::print_stdout,
        clippy::print_stderr
    )
)]

//! # Claude Agent SDK
//!
//! A Rust SDK for spawning and communicating with the Claude Code CLI.
//!
//! This crate provides an async API for:
//! - Spawning the `claude` CLI as a subprocess
//! - Communicating over NDJSON stdin/stdout
//! - Handling the bidirectional control protocol (permissions, hooks, MCP)
//! - Streaming SDK messages as an async channel
//!
//! ## Quick Start
//!
//! ```no_run
//! use claude_agent_sdk::{query, ClaudeOptions, SdkMessage};
//!
//! # async fn example() -> claude_agent_sdk::Result<()> {
//! let mut handle = query("What is 2+2?", ClaudeOptions::default()).await?;
//!
//! while let Some(msg) = handle.messages.recv().await {
//!     match msg? {
//!         SdkMessage::Assistant(assistant) => {
//!             println!("Assistant: {:?}", assistant.message.content);
//!         }
//!         SdkMessage::Result(result) => {
//!             println!("Done: {result:?}");
//!             break;
//!         }
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod error;
pub mod mcp;
pub mod query;
pub mod types;

mod protocol;
mod transport;

// Top-level re-exports for convenience
pub use client::ClaudeClient;
pub use error::{Result, SdkError};
pub use query::{query, query_streaming, QueryHandle};
pub use types::agents::AgentDefinition;
pub use types::content::{ApiMessage, ContentBlock};
pub use types::hooks::{HookDefinition, HookEvent, HookHandler, HookOutput, SyncHookOutput};
pub use types::mcp::{McpConnectionStatus, McpServerConfig, McpServerStatus, McpSetServersResult};
pub use types::messages::SdkMessage;
pub use types::options::ClaudeOptions;
pub use types::permissions::{PermissionHandler, PermissionMode, PermissionResult};
