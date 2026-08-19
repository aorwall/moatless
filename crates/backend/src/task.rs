//! Permanent facade re-exporting the `task` module from `moatless-tasks`.
//!
//! The backend stays the composition root; higher bands still in the backend
//! keep naming `crate::task::…`. Explicit named re-exports (never a glob) so
//! the crate census can resolve edges through the facade. The `test_support`
//! seeder is reached directly as `moatless_tasks::task::test_support` from the
//! db-tests that need it, so it is not surfaced here.

pub use moatless_tasks::task::{
    binding_dao, binding_summary, closer, creation, dao, error, handlers, loop_reader, models,
    presenter, relations, routes, schemas, service, statistics,
};
