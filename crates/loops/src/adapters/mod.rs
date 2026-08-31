//! What the loops band adds around the Adapter contract: the registry, the
//! durable-delivery seam, ingress, and the connection/loop plumbing the
//! providers share.
//!
//! The contract itself — the capability traits and [`AdapterError`] — is not
//! here. It lives in `moatless-adapter-model`, the Adapter kernel, so a
//! provider crate can implement an Adapter without naming this band at all.
//! Every consumer names the kernel directly; this module re-exports none of it.
//!
//! [`AdapterError`]: moatless_adapter_model::error::AdapterError
//!
//! The concrete providers and the `AppState`-taking webhook `handlers`/`routes`
//! stay in the backend above this band; they reach this module through the
//! `crate::adapters` facade and register into [`Adapters`] at the composition
//! root. Items the provider and server bands consume across that boundary are
//! `pub` here (the crate boundary widened them from `pub(crate)`); everything
//! else stays as narrow as the loops band alone requires.
pub mod common;
pub mod delivery;
pub mod events;
pub mod inbound_delivery;
pub mod ingress;
pub mod mentions;
pub mod self_echo;
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

mod registry;

pub use registry::Adapters;
