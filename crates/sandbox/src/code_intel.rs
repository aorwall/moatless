//! Go-to-definition for the UI, answered by language servers this sandbox owns.
//!
//! Separate from the agent's `LSP` tool (`init::language_servers`): Claude Code
//! starts its own server and nothing outside its CLI can reach it. A server
//! here starts on the first lookup in a workspace directory and stops after
//! 15 minutes without one.

mod lsp_client;
mod servers;

pub use servers::{CodeIntel, CodeIntelError};
