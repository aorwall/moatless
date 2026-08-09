//! Skill fetching and scanning — the implementation lives in the `moatless-workspaces`
//! crate.
//!
//! Permanent facade (see `repository.rs` for the rationale and the glob caveat).

pub use moatless_workspaces::skill::{
    error, fetcher, handlers, models, routes, scanner, schemas, service,
};
