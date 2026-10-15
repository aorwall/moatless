//! Language servers this sandbox owns, shared by the UI's go-to-definition and
//! the agent's `LSP` tool.
//!
//! A server starts when init finishes, or on first use in a workspace
//! directory, and stops after 15 minutes with no lookup and no agent session.
//! The agent reaches it through `moat lsp-proxy` and a socket per server
//! (`agent_socket`), so a checkout never runs two copies of rust-analyzer.

mod agent_socket;
mod lsp_client;
mod servers;

pub use agent_socket::{socket_path, spawn as spawn_shared};
pub use servers::{CodeIntel, CodeIntelError};
