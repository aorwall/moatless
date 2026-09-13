//! Permanent facade re-exporting the Adapter core from `moatless-loops`.
//!
//! Concrete providers and webhook `handlers`/`routes` live here, above the
//! loops band. Other re-exports here must be explicit and named, never a glob,
//! so the crate census resolves edges through this facade.
//!
//! The Adapter contract and shared vocabulary are named from their owning crate
//! (`moatless-adapter-model`, `moatless-types`) directly, not re-exported here.

pub mod handlers;
pub mod operations;
pub mod providers;
pub mod routes;

pub use moatless_loops::adapters::{
    access, delivery, events, inbound_delivery, ingress, projection, self_echo, subject_events,
};

/// Capability-matrix tests for the registry — they build the concrete
/// providers, which live in this crate above the loops band, and call the
/// kernel registry's accessors directly.
#[cfg(test)]
mod registry_matrix_tests;
