// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Workspaces band (band 4): workspace git sync, repository CRUD and token access,
//! skill fetching, plugin activation/sync, and credential storage.
//!
//! The backend composes this crate through permanent facade modules
//! (`backend/src/{workspace,repository,skill,plugin,credentials}`) that re-export
//! these paths, so call sites keep naming `crate::repository::…` etc. The facade is
//! not transitional: the backend stays the composition root that wires these ports
//! into `AppState`. Every router here is generic over the state `S` with
//! `FromRef<S>` bounds, so this crate names no concrete `AppState` — the `FromRef`
//! impls live in the backend.

pub mod credentials;
pub mod plugin;
pub mod repository;
pub mod skill;
pub mod workspace;
