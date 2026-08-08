//! Permanent facade re-exporting the `file_storage` module from
//! `moatless-tasks`.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade.

pub use moatless_tasks::file_storage::{dao, error, handlers, models, routes, s3, schemas, store};
