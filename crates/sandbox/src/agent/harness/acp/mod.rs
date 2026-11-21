//! Agent Client Protocol harness: one persistent ACP agent process per Member,
//! spoken to as JSON-RPC over stdio.
//!
//! The conversation of record is a Moatless journal (`journal.rs`); the agent's
//! own session is bound to it and restored with `session/load`, or reseeded
//! from the journal when it cannot be.
mod agent;
mod config;
mod gateway;
mod journal;
mod process;
mod rpc;

pub use config::AcpConfig;
pub(crate) use process::AcpProcess;
