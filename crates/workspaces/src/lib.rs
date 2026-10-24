// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Workspaces band (band 4): workspace git sync, repository CRUD and token
//! access, skill fetching, and plugin activation/sync.
//!
//! The backend re-exports these paths through permanent facade modules
//! (`backend/src/{workspace,repository,skill,plugin}`), so call sites keep
//! naming `crate::repository::…` etc. The facade is not transitional — the
//! backend stays the composition root wiring these ports into `AppState`.

pub mod compose;
#[cfg(all(test, feature = "db-tests"))]
mod fake_github;
pub mod plugin;
pub mod ports;
pub mod repository;
pub mod skill;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod workspace;
