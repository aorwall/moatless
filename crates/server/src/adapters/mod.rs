//! Permanent facade re-exporting the Adapter core from `moatless-loops`.
//!
//! The backend stays the composition root. The concrete providers and the
//! `AppState`-taking webhook `handlers`/`routes` live here, above the loops
//! band, and register into
//! [`Adapters`](moatless_adapter_model::registry::Adapters) at the root;
//! everything else in this namespace is re-exported from the crate so the
//! provider and server bands keep naming `crate::adapters::…` unchanged.
//! Explicit named re-exports (never a glob) so the crate census resolves edges
//! through the facade.
//!
//! Only what the loops band itself owns is re-exported here. The Adapter
//! *contract* — the capability traits, `AdapterError`, and the registry — belongs
//! to `moatless-adapter-model`, and the shared vocabulary (`AdapterKind`,
//! `BindingDestination`, `DestinationPostResult`) to `moatless-types`; both are
//! named from their owning crate directly. Routing a type through two facades
//! would give it three paths, and the longest one wins by sitting closest to
//! the caller.

pub mod handlers;
pub mod operations;
pub mod providers;
pub mod routes;

pub use moatless_loops::adapters::{
    access, delivery, events, inbound_delivery, ingress, projection, self_echo, subject_events,
};

/// Capability-matrix tests for the registry — they build the concrete providers,
/// which live in this crate above the loops band, and call the kernel registry's
/// accessors directly.
#[cfg(test)]
mod registry_matrix_tests;
