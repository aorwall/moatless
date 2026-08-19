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

pub mod access_control;
// Both feeds allocate and read positions, so the whole module needs `sqlx`.
#[cfg(feature = "database")]
pub mod change_feed;
#[cfg(feature = "database")]
pub mod db_enum;
pub mod sandbox;
pub mod ui_rpc;
