//! Task event handler — background worker that processes agent lifecycle
//! events.
//!
//! Reads the task-event change feed and handles agent.completed, agent.stopped,
//! agent.error and task.metadata.updated events, and hands `message.*` events
//! to the Adapters subscribed to them.

pub mod agent_fallback;
pub mod agent_handlers;
pub mod dispatcher;
pub mod metrics;
pub mod models;
pub mod pricing;

pub use dispatcher::{spawn_change_pruner, spawn_event_handler};
