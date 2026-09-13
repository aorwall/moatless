// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Sandbox-control (band 6): the orchestrator that drives a sandbox runtime.
//!
//! What a runtime *implements* — the mutation and observation contract — and
//! the machinery every runtime reuses live a band below, in
//! `moatless-sandbox-runtime`. This crate depends on it and re-exports nothing
//! from it: a caller that wants the contract names that crate.
//!
//! The two container runtimes (`sandbox-k8s`, `sandbox-docker`) live a band
//! above and depend downward on this crate; nothing here names `kube` or
//! `bollard`. The backend composes this crate through the permanent facade at
//! `crates/server/src/sandbox/mod.rs`, which re-exports these modules (`k8s`
//! and `docker` as crate aliases) so the higher bands still in the backend
//! keep naming `crate::sandbox::…`. The facade is not transitional: the
//! backend stays the composition root that wires these ports into `AppState`.
//! Every router here is generic over the state `S` with `FromRef<S>` bounds,
//! so this crate names no concrete `AppState` — the `FromRef` impls live in
//! the backend.
//!
//! Reach the workspace domain only through the ports `moatless-ports`
//! declares, and identity only through `CommitIdentity`, `HarnessCredentials`,
//! `SecretResolver` and `GitHostTokenIssuer`. Neither `moatless-identity` nor
//! `moatless-workspaces` is a `[dependencies]` entry — adding either re-opens
//! the edge those ports exist to close. `moatless-workspaces` is a
//! dev-dependency, for the one `db-tests` test that assembles `ProvisionDeps`
//! from a pool.

pub mod auto_provision;
pub mod compose;
pub mod event_dedup;
pub mod file_handlers;
pub mod handlers;
pub mod lifecycle;
pub mod notify;
pub mod preflight;
pub mod routes;
pub mod secrets;
pub mod workspace_repository;
pub mod workspace_snapshot;

pub mod access;
pub mod server_config;
pub mod state;
