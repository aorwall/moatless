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

//! Drives the Claude Code CLI: spawns `claude` as a subprocess or a tmux TUI,
//! streams its NDJSON messages over a channel, and answers its permission
//! requests over the control protocol. `examples/simple_query.rs` shows a
//! one-shot [`query`].

pub mod api;
mod arg_limits;
mod config_dir;
pub mod error;
pub mod query;
pub mod tmux;

mod protocol;
mod session_cost;
mod transcripts;
mod transport;

pub use api::content::{ApiMessage, ContentBlock};
pub use api::hooks::HookEvent;
pub use api::mcp::{McpConnectionStatus, McpServerConfig, McpServerStatus, McpSetServersResult};
pub use api::messages::{MessagePriority, SdkMessage};
pub use api::options::ClaudeOptions;
pub use api::permissions::{PermissionHandler, PermissionMode, PermissionResult};
pub use error::{Result, SdkError};
pub use query::{QueryHandle, query, query_streaming};
pub use tmux::{has_resumable_transcript, query_tmux, query_tmux_streaming, transcript_dir};
pub use transcripts::project_dir_name;
