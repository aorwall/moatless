//! What the loops band adds around the Adapter contract: the durable-delivery
//! seam, ingress, and the connection/loop plumbing the providers share.
//!
//! The contract itself — the capability traits, [`AdapterError`], and the
//! registry that maps a kind to a capability — is not here. It lives in
//! `moatless-adapter-model`, the Adapter kernel, so a provider crate can
//! implement an Adapter without naming this band at all. Every consumer names
//! the kernel directly; this module re-exports none of it.
//!
//! [`AdapterError`]: moatless_adapter_model::error::AdapterError
//!
//! The concrete providers and the `AppState`-taking webhook `handlers`/`routes`
//! stay in the backend above this band; they reach this module through the
//! `crate::adapters` facade and register into
//! [`Adapters`](moatless_adapter_model::registry::Adapters) at the composition
//! root. Items the provider and server bands consume across that boundary are
//! `pub` here (the crate boundary widened them from `pub(crate)`); everything
//! else stays as narrow as the loops band alone requires.
pub mod access;
pub mod projection;
pub mod delivery;
pub mod events;
pub mod inbound_delivery;
pub mod ingress;
pub mod self_echo;
pub mod subject_events;
