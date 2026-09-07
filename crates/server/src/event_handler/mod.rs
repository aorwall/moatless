//! Task event handler — background worker that processes agent lifecycle events.
//!
//! Subscribes to Redis streams and handles agent.completed, agent.stopped,
//! agent.error, agent.message (auto-tagging), and task.metadata.updated events.

pub mod agent_handlers;
pub mod consumer;
pub mod metrics;
pub mod models;
pub mod pricing;
pub mod skeleton;

pub use consumer::spawn_event_handler;
