//! The durable record that the environment started a terminal session on a
//! Task.

mod dao;
mod position;

pub use dao::{current_position, prune_starts, record_start, starts_after};
pub use position::{CHANGES_PER_POLL, RETAINED_CHANGES, TerminalSessionStart};
