//! Plugin activation and sync — the implementation lives in the `moatless-workspaces`
//! crate.
//!
//! Permanent facade (see `repository.rs` for the rationale and the glob caveat).

pub use moatless_workspaces::plugin::{
    activation, dao, error, handlers, models, routes, schemas, sync,
};
