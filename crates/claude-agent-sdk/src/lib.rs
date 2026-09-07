// Allow unwrap/expect/print in test code — production code still enforces the deny.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::type_complexity,
        clippy::match_wild_err_arm
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
pub mod config_dir;
pub mod error;
pub mod mcp;
pub mod query;
pub mod tmux;
pub mod api;

mod protocol;
mod transport;
mod transcripts;

// Top-level re-exports for convenience
pub use client::ClaudeClient;
pub use config_dir::resolve_claude_config_dir;
pub use error::{Result, SdkError};
pub use query::{QueryHandle, query, query_streaming};
pub use tmux::{has_resumable_transcript, query_tmux, query_tmux_streaming, transcript_dir};
pub use api::agents::AgentDefinition;
pub use api::content::{ApiMessage, ContentBlock};
pub use api::hooks::{HookDefinition, HookEvent, HookHandler, HookOutput, SyncHookOutput};
pub use api::mcp::{McpConnectionStatus, McpServerConfig, McpServerStatus, McpSetServersResult};
pub use api::messages::{MessagePriority, SdkMessage};
pub use api::options::ClaudeOptions;
pub use api::permissions::{PermissionHandler, PermissionMode, PermissionResult};
