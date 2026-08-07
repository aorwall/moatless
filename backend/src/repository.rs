//! Repository CRUD, config sync, and GitHub token access — the implementation lives
//! in the `moatless-workspaces` crate.
//!
//! Permanent facade, not transitional: the backend stays the composition root that
//! wires these ports into `AppState` (the routers are generic over `S: FromRef`, so
//! the crate names no `AppState`). Re-exports are explicit and never a glob — the
//! census drops a glob-through-facade into UNRESOLVED, so a `pub use …::*` here
//! would make `crate::repository::<submodule>::…` unresolvable to the ratchet.

pub use moatless_workspaces::repository::{
    access, config_reader, dao, error, handlers, models, routes, schemas, seed,
};
