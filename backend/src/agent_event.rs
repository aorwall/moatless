//! Permanent facade re-exporting the `agent_event` module from `moatless-tasks`.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade.

pub use moatless_tasks::agent_event::{is_agent_message, AgentEvent, EventKey};
