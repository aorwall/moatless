//! Adapter trait, registry, connection storage, and webhook routes.
pub mod common;
pub mod dao;
pub mod delivery;
pub mod error;
pub mod footer;
pub mod handlers;
pub(crate) mod inbound_delivery;
pub(crate) mod ingress;
pub mod metrics;
pub mod models;
pub mod providers;
pub mod routes;
pub mod signature;
pub(crate) mod subject_events;
/// Loopback stubs for Adapter HTTP clients.
///
/// Reachable from `backend/tests/` as well as from in-crate tests, because the
/// workflows whose API-call budgets need pinning take an `AppState` and no
/// in-crate test can build one. Both tiers stubbing the same way is the point:
/// a second copy of this in `backend/tests/common/` would drift, and a drifted
/// stub is a test that quietly reaches the real sender. Never compiled into a
/// release build — `db-tests` is a test-only feature.
#[cfg(any(test, feature = "db-tests"))]
pub mod test_support;

mod capabilities;
mod registry;

pub use capabilities::{BindingDestination, DestinationPostResult};
pub use moatless_api_schemas::enums::AdapterKind;
pub use providers::{github_pr, linear, slack, telegram, webhook};
pub use registry::Adapters;

pub(crate) use capabilities::{
    Adapter, AdapterBindingReply, AdapterBindingStatus, AdapterDestinationBuilder,
    AdapterDirectDestination, AdapterDirectMessage, AdapterInboundPresentation,
    AdapterMessageReply, AdapterNewThread, AdapterOutboundContent, AdapterPostBindingFollowup,
    AdapterReactions, AdapterUserIdentity,
};
