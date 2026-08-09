//! Permanent facade re-exporting the `feedback` module from `moatless-tasks`.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade.

pub use moatless_tasks::feedback::{dao, error, handlers, models, routes, schemas};
