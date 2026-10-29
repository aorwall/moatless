//! Loops band: loops, bindings, and the Adapter core that no single Adapter
//! owns.
//!
//! The backend is the composition root and reaches this crate through the
//! permanent facades `crate::adapters`, `crate::bindings` and `crate::loops`,
//! so callers keep naming those paths unchanged. The concrete providers
//! (`github_pr`, `slack`, `telegram`, `linear`, `webhook`) and the
//! `AppState`-taking `handlers`/`routes` register into it from the root above.

// Allow unwrap/expect in test code — production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod adapters;
pub mod bindings;
pub mod compose;
pub mod loops;
pub mod rooms;
