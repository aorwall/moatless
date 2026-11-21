// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Identity band (band 3): authentication, provider-token credentials,
//! adapter-app registry, secret storage, and first-run setup.
//!
//! Request extractors live in `moatless_core::http_auth`; this crate only
//! implements `UserTokenVerifier`'s caller side.
//!
//! The backend composes this crate through permanent facade modules that
//! re-export its paths, staying the composition root that wires `AppState`.
//! Adapter installation belongs to the provider crate, not here; user login
//! and git-host credentials stay identity's.

pub mod adapter_apps;
pub mod auth;
pub mod compose;
pub mod credentials;
pub mod forgejo;
pub mod git_credential_overrides;
pub mod git_host_access;
pub mod git_host_tokens;
pub mod ports;
pub mod secret;
pub mod setup;
