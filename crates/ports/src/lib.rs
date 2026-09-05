//! Cross-crate ports and their supporting contract types.

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
pub mod identity;
pub mod sandbox;
pub mod task;
pub mod workspace;
