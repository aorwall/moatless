//! Interactive terminal runtime contracts and PTY ownership.

mod models;
mod runtime;

pub use models::*;
pub use runtime::{AttachedTerminal, TerminalError, TerminalManager};
