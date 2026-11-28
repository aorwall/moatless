#![cfg_attr(
    test,
    expect(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )
)]

//! What a sandbox runtime implements, and the machinery every runtime reuses.
//!
//! The **contract** is [`runtime::SandboxRuntime`] (mutation) and
//! [`fleet::FleetObserver`] (observation) plus shared values like
//! [`runtime::RuntimeOutcome`] and [`convergence::SandboxObservedStatus`]; the
//! **machinery** is the gRPC client, status reads, storage plan, response
//! schemas, and ingress counters.
//!
//! The orchestrator (`Lifecycle`, the reconciler, the watcher) lives a band
//! above in `moatless-sandbox-control` and never reaches down into this crate.

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
pub mod status_reader;
pub mod storage;

pub mod observation_events;

pub mod file_conversion;
pub mod workspace_copy;
