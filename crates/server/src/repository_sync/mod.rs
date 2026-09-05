//! Repository config-sync — a cross-band HTTP module that composes a
//! repository's loop sync (`loops`) with its git-declared workspace sync
//! (`workspaces`) synchronously. Server-band, owning its own route and `ApiDoc`;
//! the same shape as `task_connections`.
//!
//! It lives here rather than in `repository` because naming `loops`
//! (`LoopSyncService`) from the `repository` (`workspaces`) band is an upward
//! edge that would trap the module and its router above their band. The server
//! band is the one place allowed to name up into `loops`, and the admin-
//! visibility contract requires the loop-sync failure to surface synchronously,
//! so this is a composition, not an event publish.

pub mod handlers;
pub mod routes;
