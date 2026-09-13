//! SSE event streaming module.
//!
//! Reads the task-event change feed and fans it out to connected SSE clients
//! via the shared SSE event bus.

pub mod feed;
pub mod handlers;
pub mod routes;
mod state;
