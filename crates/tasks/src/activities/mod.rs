//! What *happened* in a conversation, as rows.
//!
//! Everything besides a Message (role + text): a tool call and its outcome,
//! thinking, a Sandbox status or error — identified by where it came from
//! rather than when written, so redelivery is a no-op.
//!
//! The counter and change log live here too: only Turn transitions enter the
//! log, so a Message arriving does not, and no T3 vocabulary appears here.

pub(crate) mod dao;
pub mod mapping;
pub(crate) mod outputs;
pub mod pruner;
pub mod view;

pub use moatless_types::activity::Activity;
pub use pruner::spawn_change_pruner;
