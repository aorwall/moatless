// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Sandbox-control (band 6): the orchestrator that drives a sandbox runtime.
//!
//! The mutation/observation contract lives a band below in
//! `moatless-sandbox-runtime` (re-exported from nowhere here); the container
//! runtimes live a band above, depending down, and this crate names neither
//! `kube` nor `bollard`. The backend's permanent facade
//! (`crates/server/src/sandbox`) is the composition root wiring `AppState`.
//!
//! Reach the workspace only via `moatless-ports` and identity only via its
//! named traits: `moatless-identity`/`moatless-workspaces` aren't real deps.

pub mod auto_provision;
pub mod compose;
pub mod event_dedup;
pub mod file_handlers;
pub mod handlers;
pub mod lifecycle;
pub mod notify;
pub mod observation;
pub mod preflight;
pub mod routes;
pub mod secrets;
pub mod workspace_repository;
pub mod workspace_snapshot;

pub mod access;
pub mod state;
pub mod status;
