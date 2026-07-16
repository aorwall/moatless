//! Pure policy and shared core types for the Moatless backend.

// Test code may unwrap when asserting serialization behavior.
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod scope;
