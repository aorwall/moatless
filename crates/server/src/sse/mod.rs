//! SSE event streaming module.
//!
//! Consumes events from Redis Streams and fans them out to connected SSE
//! clients via the shared SSE event bus.

pub mod consumer;
pub mod handlers;
pub mod routes;
mod state;
