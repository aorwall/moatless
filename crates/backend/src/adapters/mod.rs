//! Permanent facade re-exporting the Adapter core from `moatless-loops`.
//!
//! The backend stays the composition root. The concrete providers and the
//! `AppState`-taking webhook `handlers`/`routes` live here, above the loops
//! band, and register into [`Adapters`] at the root; everything else in this
//! namespace is re-exported from the crate so the provider and server bands keep
//! naming `crate::adapters::…` unchanged. Explicit named re-exports (never a
//! glob) so the crate census resolves edges through the facade.
//!
//! Only what the loops band itself owns is re-exported here. The Adapter
//! *contract* — the capability traits and `AdapterError` — belongs to
//! `moatless-adapter-model`, and the shared vocabulary (`AdapterKind`,
//! `BindingDestination`, `DestinationPostResult`) to `moatless-types`; both are
//! named from their owning crate directly. Routing a type through two facades
//! would give it three paths, and the longest one wins by sitting closest to
//! the caller.

pub mod handlers;
pub mod providers;
pub mod routes;

pub use moatless_loops::adapters::{
    Adapters, common, delivery, events, inbound_delivery, ingress, self_echo, subject_events,
};

/// Loopback stubs for Adapter HTTP clients, re-exported from the crate under the
/// same gate the backend's tests reach them by. Never in a release build.
#[cfg(any(test, feature = "db-tests"))]
pub use moatless_loops::adapters::test_support;

/// Capability-matrix tests for the registry — they build the concrete providers
/// (which live in this crate, above the loops band) and assert the registry's
/// `pub(crate)` accessors through `test_support::registry`.
#[cfg(test)]
mod registry_matrix_tests;
