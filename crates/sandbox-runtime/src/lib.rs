// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! What a sandbox runtime implements, and the machinery every runtime reuses.
//!
//! Two things live here. The **contract** is what a container runtime must
//! satisfy: [`runtime::SandboxRuntime`] for mutation, [`fleet::FleetObserver`]
//! for observation, and the values both sides speak in — a
//! [`runtime::RuntimeOutcome`], an [`operation::OperationGeneration`], a
//! [`convergence::SandboxObservedStatus`], a [`provisioning::ProvisionRequest`]. The
//! **machinery** is everything a runtime needs to answer with: the gRPC client
//! and its error classification, the status reads, the storage plan, the
//! response schemas, and the ingress counters.
//!
//! The orchestrator that drives a runtime — `Lifecycle`, the reconciler, the
//! watcher, and the durable intent behind them — lives in
//! `moatless-sandbox-control`, a band above. Nothing here may reach it.

pub mod access;
pub mod classify;
pub mod client;
pub mod container_status;
pub mod convergence;
pub mod error;
pub mod fleet;
pub mod live_status;
pub mod marker;
pub mod metrics;
pub mod operation;
pub mod outcome;
pub mod provisioning;
pub mod repo_config_encoding;
pub mod runtime;
pub mod schemas;
pub mod server_runner;
pub mod status_reader;
pub mod storage;

pub mod observation_events;

pub mod file_conversion;
