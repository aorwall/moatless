//! Agent command and acknowledgement channel.

mod channel;
pub(crate) use crate::agent::compose::{
    AGENT_TYPE_CLAUDE_CODE, AGENT_TYPE_CODEX, resolve_agent_harness,
};
pub use crate::agent::compose::{build_harness, spawn_agent};
pub(crate) use channel::AgentCommand;
pub use channel::{AgentHandle, RunAck};
