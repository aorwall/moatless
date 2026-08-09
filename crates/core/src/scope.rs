//! Temporary compatibility re-export for scope policy consumers.
//!
//! New production code should import scope policy from
//! [`moatless_backend_core::scope`]. This module keeps the existing
//! `moatless_core::scope::*` path valid for consumers that have not moved yet.

pub use moatless_backend_core::scope::*;
