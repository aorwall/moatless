//! What the loops band adds around the Adapter contract: the durable-delivery
//! seam, ingress, and the connection/loop plumbing the providers share.
//!
//! The contract itself — capability traits, `AdapterError`, and the kind
//! registry — lives in `moatless-adapter-model`, the Adapter kernel; every
//! consumer names the kernel directly, and this module re-exports none of it.
//!
//! Concrete providers and webhook `handlers`/`routes` stay above this band,
//! reaching in through the `crate::adapters` facade.
pub mod access;
pub mod delivery;
pub mod events;
pub mod inbound_delivery;
pub mod ingress;
pub mod projection;
pub mod self_echo;
pub mod subject_events;
