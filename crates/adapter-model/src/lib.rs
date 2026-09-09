// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! The Adapter kernel: the whole Adapter contract in one crate.
//!
//! What an Adapter offers ([`capabilities`]), what it speaks ([`apps`],
//! [`footer`], [`signature`]) and the access it gates on ([`access`]), what it
//! can fail with ([`error`]), which external account it speaks for
//! ([`connections`]), how it hands inbound work over durably ([`delivery`]),
//! and how a delivery ends ([`ingress`]).
//! A provider crate implements the contract by naming this crate and nothing
//! above it; the bands that orchestrate providers name the same items from the
//! same paths.
//!
//! Two features keep the base surface to the contract itself: `postgres` adds
//! the connection store and its row shapes, `http` adds the `IntoResponse`
//! rendering of [`error::AdapterError`]. A consumer that neither reads rows nor
//! answers requests compiles neither a database driver nor axum.

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

/// Helpers the Adapter error tests need, shared with the crates whose own tests
/// assert against a rendered [`error::AdapterError`]. The `test,` disjunct stays
/// FIRST so items gated this way read as test-context. Requires `http`, which
/// `test-support` enables: two of the three helpers are about the renderer.
#[cfg(all(feature = "http", any(test, feature = "test-support")))]
pub mod test_support;

pub mod outbound;
