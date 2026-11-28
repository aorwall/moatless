#![cfg_attr(
    test,
    expect(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )
)]

//! The Adapter kernel: the whole Adapter contract in one crate.
//!
//! What an Adapter offers ([`capabilities`]), what it speaks ([`apps`],
//! [`footer`], [`signature`]), its access gate ([`access`]), failure type
//! ([`error`]), the account it speaks for ([`connections`]), durable inbound
//! handoff ([`delivery`]), and how a delivery ends ([`ingress`]). A provider
//! crate names this crate and nothing above it.
//!
//! `postgres` adds the connection store; `http` adds `IntoResponse` rendering
//! of [`error::AdapterError`], each optional so a consumer skips that driver.

pub mod access;
pub mod apps;
pub mod binding;
pub mod capabilities;
#[cfg(feature = "postgres")]
pub mod compose;
pub mod connections;
pub mod delivery;
pub mod error;
pub mod footer;
pub mod ingress;
pub mod loops;
pub mod mentions;
pub mod metrics;
pub mod registry;
pub mod signature;

/// Helpers the Adapter error tests need, shared across crates.
///
/// Used by any crate whose tests assert against a rendered
/// [`error::AdapterError`]. The `test,` disjunct stays FIRST so items gated
/// this way read as test-context; requires `http`, which `test-support`
/// enables.
#[cfg(all(feature = "http", any(test, feature = "test-support")))]
pub mod test_support;

pub mod outbound;
