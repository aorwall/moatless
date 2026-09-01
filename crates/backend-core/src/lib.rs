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
pub mod activity;
pub mod binding;
// Both feeds allocate and read positions, so the whole module needs `sqlx`.
#[cfg(feature = "database")]
pub mod change_feed;
#[cfg(feature = "database")]
pub mod db_enum;
pub mod identity;
pub mod running;
pub mod sandbox;
/// The error every published store seam returns. Defined in `moatless_types`
/// so the floor can name it; re-exported here because the ports do.
pub mod store {
    pub use moatless_types::store::StoreError;
}
pub mod task;
pub mod ui_rpc;
pub mod workspace;
