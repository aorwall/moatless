//! Provider-token credential persistence and provider refresh.
//!
//! The `provider_tokens` store stays private to this crate: a peer band takes a
//! [`CredentialService`] handle from [`crate::compose::credentials`] and calls
//! its operations.

pub mod codex;
pub mod error;
mod models;
mod service;
// `test` stays first so the census keeps scoring this fake as test-context; the
// feature exposes it to other crates' tests (see Cargo.toml `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use error::CredentialError;
pub use models::{StoreOutcome, WriteSource, should_broadcast};
pub use service::CredentialService;
