//! # Codex SDK
//!
//! A Rust SDK for programmatically interacting with the Codex coding agent.
//!
//! This crate provides an async API mirroring the Claude Agent SDK's interface,
//! but backed by Codex's in-process app-server runtime for full protocol access
//! (streaming deltas, approval handling, hook observation).
//!
//! ## Quick Start
//!
//! ```no_run
//! use codex_sdk::{query, CodexOptions, CodexMessage};
//!
//! # async fn example() -> codex_sdk::Result<()> {
//! let mut handle = query("What files are in this directory?", CodexOptions::default()).await?;
//!
//! while let Some(msg) = handle.messages.recv().await {
//!     match msg? {
//!         CodexMessage::AgentMessageDelta(delta) => {
//!             print!("{}", delta.delta);
//!         }
//!         CodexMessage::TurnCompleted(_) => break,
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod approvals;
pub mod client;
pub mod error;
pub mod hooks;
pub mod messages;
pub mod options;
pub mod query;

mod runtime;

// Top-level re-exports for convenience.
pub use approvals::{ApprovalHandler, AutoApproveHandler, DenyAllHandler};
pub use client::CodexClient;
pub use error::{Result, SdkError};
pub use messages::CodexMessage;
pub use options::CodexOptions;
pub use query::{QueryHandle, query, query_streaming};

// Re-export commonly used protocol types so SDK users don't need to depend
// on codex-app-server-protocol directly.
pub use codex_app_server_protocol::AskForApproval;
pub use codex_app_server_protocol::CommandExecutionApprovalDecision;
pub use codex_app_server_protocol::CommandExecutionRequestApprovalParams;
pub use codex_app_server_protocol::FileChangeApprovalDecision;
pub use codex_app_server_protocol::FileChangeRequestApprovalParams;
pub use codex_app_server_protocol::PermissionsRequestApprovalParams;
pub use codex_app_server_protocol::SandboxMode;
