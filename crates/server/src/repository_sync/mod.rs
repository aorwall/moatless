//! Repository config-sync: a cross-band module.
//!
//! Composes a repository's loop sync (`loops`) with its git-declared workspace
//! sync (`workspaces`), on an administrator's request or when the default
//! branch moves. Server-band, owning its own route and `ApiDoc`.
//!
//! Lives here, not in `repository`: naming `loops` from the `repository` band
//! is an upward edge, and the admin-visibility contract needs the loop-sync
//! failure to surface synchronously — a composition, not an event publish.

pub mod handlers;
pub mod routes;

mod sync;
pub mod watch;
