//! Workspace git sync — the implementation lives in the `moatless-workspaces` crate.
//!
//! Permanent facade (see `repository.rs` for the rationale and the glob caveat).
//! Re-exports the `workspace` submodules the backend composes; the census resolves
//! `crate::workspace::<submodule>::…` through these explicit re-exports.

pub use moatless_workspaces::workspace::{dao, error, git_sync, handlers, models, routes, schemas};
