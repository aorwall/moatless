// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Identity band (band 3): authentication, provider-token credentials, team
//! membership, adapter-app registry, secret storage, and first-run setup.
//!
//! The request extractors are axum plumbing over two ports, not identity, and
//! live in `moatless_core::http_auth`; this crate implements
//! `UserTokenVerifier`'s caller side of the middleware and nothing more.
//!
//! The backend composes this crate through permanent facade modules
//! (`backend/src/{auth,team,adapter_apps,secret,setup}`) that re-export
//! these paths, so call sites keep naming `crate::auth::…` etc. The facade is not
//! transitional: the backend stays the composition root that wires identity into
//! `AppState`.
//!
//! Adapter installation belongs to the provider crate: an Adapter resolves its
//! stored credentials through `AdapterAppReader`, declared in
//! `moatless-adapter-model`. Adding an Adapter must not edit this crate. User
//! login and git-host credentials remain identity responsibilities.

pub mod adapter_apps;
pub mod auth;
pub mod compose;
pub mod credentials;
pub mod forgejo;
pub mod git_host_access;
pub mod git_host_tokens;
pub mod ports;
pub mod secret;
pub mod setup;
pub mod team;
