// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Identity band (band 3): authentication, request extractors, team membership,
//! adapter-app registry, secret storage, and first-run setup.
//!
//! The backend composes this crate through permanent facade modules
//! (`backend/src/{auth,extractors,team,adapter_apps,secret,setup}`) that re-export
//! these paths, so call sites keep naming `crate::auth::…` etc. The facade is not
//! transitional: the backend stays the composition root that wires identity into
//! `AppState` (`FromRef<AppState> for Arc<dyn extractors::UserTokenVerifier>`).

pub mod adapter_apps;
pub mod auth;
pub mod extractors;
pub mod git_host_access;
pub mod git_host_repo;
pub mod git_host_tokens;
pub mod ports;
pub mod secret;
pub mod setup;
pub mod team;
