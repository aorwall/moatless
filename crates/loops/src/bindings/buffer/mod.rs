//! Buffering inbound Adapter events so a burst reaches a Task as one message.
//!
//! Routing holds an event for its Binding's window instead of delivering it,
//! commits it to `inbound_message_buffer`, and reports the durable delivery
//! succeeded. When the window closes the flusher hands the whole binding's
//! events to routing as one combined turn.
//!
//! The window is fixed by the event that opened it, so latency is bounded at
//! exactly the window rather than starved by a steady stream.

mod combine;
mod worker;

pub(crate) mod dao;
pub(crate) mod policy;

pub use worker::MessageBufferWorker;

pub(crate) use combine::{Part, combine};
