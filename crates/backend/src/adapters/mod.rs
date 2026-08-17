//! Permanent facade re-exporting the Adapter core from `moatless-loops`.
//!
//! The backend stays the composition root. The concrete providers and the
//! `AppState`-taking webhook `handlers`/`routes` live here, above the loops
//! band, and register into [`Adapters`] at the root; everything else in this
//! namespace is re-exported from the crate so the provider and server bands keep
//! naming `crate::adapters::…` unchanged. Explicit named re-exports (never a
//! glob) so the crate census resolves edges through the facade.

pub mod handlers;
pub mod providers;
pub mod routes;

pub use moatless_loops::adapters::{
    common, delivery, error, events, inbound_delivery, ingress, metrics, self_echo, signature,
    subject_events, Adapter, AdapterBindingReply, AdapterBindingStatus, AdapterDestinationBuilder,
    AdapterDirectDestination, AdapterDirectMessage, AdapterInboundPresentation, AdapterKind,
    AdapterMessageReply, AdapterOutboundContent, AdapterPostBindingFollowup, AdapterReactions,
    Adapters, BindingDestination, DestinationPostResult, dao, footer, models,
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
