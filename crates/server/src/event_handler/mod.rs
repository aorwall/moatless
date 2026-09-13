//! Task event handler — background worker that processes agent lifecycle
//! events.
//!
//! Reads the task-event change feed and handles agent.completed, agent.stopped,
//! agent.error, agent.message (auto-tagging), and task.metadata.updated events.

pub mod agent_handlers;
pub mod dispatcher;
pub mod metrics;
pub mod models;
pub mod pricing;
pub mod skeleton;

pub use dispatcher::{spawn_change_pruner, spawn_event_handler};
