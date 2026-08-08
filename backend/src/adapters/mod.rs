//! Adapter trait, registry, connection storage, and webhook routes.
pub mod common;
pub mod delivery;
pub mod error;
pub mod handlers;
pub(crate) mod inbound_delivery;
pub(crate) mod ingress;
pub mod metrics;
pub mod providers;
pub mod routes;
pub mod self_echo;
pub mod signature;
pub(crate) mod subject_events;
/// Loopback stubs for Adapter HTTP clients.
///
/// Reachable from `backend/tests/` as well as from in-crate tests, because the
/// workflows whose API-call budgets need pinning take an `AppState` and no
/// in-crate test can build one. Both tiers stubbing the same way is the point:
/// a second copy of this in `backend/tests/integration/common/` would drift, and a drifted
/// stub is a test that quietly reaches the real sender. Never compiled into a
/// release build — `db-tests` is a test-only feature.
#[cfg(any(test, feature = "db-tests"))]
pub mod test_support;

mod capabilities;
mod registry;

pub use capabilities::{BindingDestination, DestinationPostResult};
pub use moatless_api_schemas::enums::AdapterKind;

// Adapter domain types, `adapter_connections` persistence, and the pure footer
// render/parse layer live in the `moatless-adapter-model` crate. This facade
// lets the call sites in `adapters` and `providers` keep naming them through
// `crate::adapters::{models, dao, footer}`, where reaching them through the
// adapters namespace is natural. Transitional: it comes out when `adapters`
// itself becomes its own crate (Steps 8–11), at which point those call sites
// move with it. `bindings` names the crate directly and does not route through
// this facade. The footer's DAO-loading wrapper `task_comment_footer` is not
// here — it lives in `bindings::footer`, where the port belongs.
pub use moatless_adapter_model::{dao, footer, models};
pub use registry::Adapters;

pub(crate) use capabilities::{
    Adapter, AdapterBindingReply, AdapterBindingStatus, AdapterDestinationBuilder,
    AdapterDirectDestination, AdapterDirectMessage, AdapterInboundPresentation,
    AdapterMessageReply, AdapterNewThread, AdapterOutboundContent, AdapterPostBindingFollowup,
    AdapterReactions,
};
