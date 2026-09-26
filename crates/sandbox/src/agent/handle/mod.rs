//! Agent command and acknowledgement channel.

mod channel;
pub(crate) use crate::agent::compose::resolve_agent_harness;
pub use crate::agent::compose::{build_harness, spawn_agent};
pub(crate) use channel::AgentCommand;
pub use channel::{AgentHandle, RunAck};
