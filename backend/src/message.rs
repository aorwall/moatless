//! Permanent facade re-exporting the `message` module from `moatless-tasks`.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade.

pub use moatless_tasks::message::{
    contexts, dao, enricher, error, handlers, hooks, models, routes, schemas, send, service,
    transform,
};
