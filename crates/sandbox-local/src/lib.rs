//! Local-process sandbox provider (`SANDBOX_PROVIDER=local`): each task's
//! sandbox is a child process of the backend, on the same host, for isolated
//! E2E runs (`scripts/local-e2e`).
//!
//! The registry lives in memory: a backend that dies without killing its
//! process group leaves orphaned sandboxes, and a restarted one has none.
//! Sandboxes still share the preview proxy port (8003; only the first binds
//! it).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod client;
mod command_builder;
pub mod fleet_observer;
mod naming;
pub mod provider;
mod status_resolver;
