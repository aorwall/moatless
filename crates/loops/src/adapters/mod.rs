//! Adapter trait, registry, connection storage, and the shared ingress core.
//!
//! The concrete providers and the `AppState`-taking webhook `handlers`/`routes`
//! stay in the backend above this band; they reach this module through the
//! `crate::adapters` facade and register into [`Adapters`] at the composition
//! root. Items the provider and server bands consume across that boundary are
//! `pub` here (the crate boundary widened them from `pub(crate)`); everything
//! else stays as narrow as the loops band alone requires.
pub mod common;
pub mod delivery;
pub mod error;
pub mod events;
pub mod inbound_delivery;
pub mod ingress;
pub mod metrics;
pub mod self_echo;
pub mod signature;
pub mod subject_events;
/// Loopback stubs for Adapter HTTP clients.
///
/// Reachable from `backend/tests/` and the backend's own tests as well as from
/// in-crate tests, because the workflows whose API-call budgets need pinning
/// take an `AppState` and no in-crate test can build one. Both tiers stubbing
/// the same way is the point: a second copy would drift, and a drifted stub is a
/// test that quietly reaches the real sender. Gated so it never compiles into a
/// release build; the `test,` disjunct stays first for the census.
#[cfg(any(test, feature = "test-support", feature = "db-tests"))]
pub mod test_support;

mod capabilities;
mod registry;

pub use capabilities::{BindingDestination, DestinationPostResult};
pub use moatless_adapter_model::models::AdapterKind;

// Adapter domain types, `adapter_connections` persistence, and the pure footer
// render/parse layer live in the `moatless-adapter-model` crate. This re-export
// lets the call sites in `adapters` and the providers keep naming them through
// `crate::adapters::{models, dao, footer}`. `bindings` names the crate directly.
// The footer's DAO-loading wrapper `task_comment_footer` is not here — it lives
// in `bindings::footer`, where the port belongs.
pub use moatless_adapter_model::{dao, footer, models};
pub use registry::Adapters;

pub use capabilities::{
    Adapter, AdapterBindingReply, AdapterBindingStatus, AdapterDestinationBuilder,
    AdapterDirectDestination, AdapterDirectMessage, AdapterInboundPresentation,
    AdapterMessageReply, AdapterOutboundContent, AdapterPostBindingFollowup, AdapterReactions,
};
