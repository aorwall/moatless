//! Permanent facade re-exporting the `agent` module from `moatless-tasks`.
//!
//! The backend stays the composition root; higher bands still in the backend
//! keep naming `crate::agent::…`. Explicit named re-exports (never a glob) so
//! the crate census can resolve edges through the facade.

pub use moatless_tasks::agent::{catalog, error, handlers, prompt, routes, schemas, service};
