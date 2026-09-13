//! The pruner that keeps the listing change feed bounded.
//!
//! The feed itself is [`moatless_core::change_feed::listing`] — it is written
//! from more than one band, so it lives below all of them.

pub mod pruner;

pub use pruner::spawn_change_pruner;
