//! The durable record that the environment opened a browser tab on a Task.

mod dao;
mod position;

pub use dao::{current_position, opens_after, prune_opens, record_open};
pub use position::{CHANGES_PER_POLL, PreviewTabOpen, RETAINED_CHANGES};
