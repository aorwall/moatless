//! Loops band: loops, bindings, and the Adapter core that no single Adapter owns.
//!
//! Lifted out of `backend/src`. The backend stays the composition root and
//! reaches this crate through permanent facade modules (`crate::adapters`,
//! `crate::bindings`, `crate::loops`) that re-export it, so the provider and
//! server bands still in the backend keep naming those paths unchanged. The
//! concrete providers (`github_pr`, `slack`, `telegram`, `linear`, `webhook`)
//! and the `AppState`-taking `handlers`/`routes` are *not* here — they sit above
//! this band and register into it at the root.

// Allow unwrap/expect in test code — production code still enforces the deny.
// The test modules lifted in with this crate rely on it, exactly as
// `identity`/`tasks`/`workspaces`/`backend` do.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod adapters;
pub mod bindings;
pub mod compose;
pub mod loops;
