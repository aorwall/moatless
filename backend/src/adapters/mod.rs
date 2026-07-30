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
#[cfg(test)]
pub(crate) mod test_support;

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
