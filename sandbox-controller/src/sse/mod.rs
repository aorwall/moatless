//! SSE event streaming module.
//!
//! Consumes events from Redis Streams and fans them out to connected
//! SSE clients via a `tokio::sync::broadcast` channel.

pub mod consumer;
pub mod routes;
