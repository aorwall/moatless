//! Buffering inbound Adapter events so a burst reaches a Task as one message.
//!
//! Routing holds an event for its Binding's window instead of delivering it,
//! commits it to `inbound_message_buffer`, and reports the durable delivery
//! succeeded. When the window closes the flusher hands the whole binding's
//! events to routing as one combined turn.
//!
//! A window closes after a quiet period with no new event, so a lone event is
//! not held for the whole window. The window, counted from the event that
//! opened it, caps that extension, so a steady stream cannot starve the flush.

mod combine;
mod worker;

pub(crate) mod dao;
pub(crate) mod policy;

pub use worker::MessageBufferWorker;

pub(crate) use combine::{Part, combine};
