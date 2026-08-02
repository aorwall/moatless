//! Shared backend policy, read vocabularies, and subsystem ports.

// Test code may unwrap when asserting serialization behavior.
#![cfg_attr(
    test,
    allow(
        clippy::disallowed_methods,
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "unit tests assert exact projections and policy cases"
    )
)]

pub mod activity;
// Device pairing stores and consumes codes, so the whole module needs `sqlx`.
#[cfg(feature = "database")]
pub mod pairing;
pub mod sandbox;
pub mod scope;
