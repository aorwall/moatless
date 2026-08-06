//! Temporary compatibility re-export for the Phase 1 backend-core extraction.
//!
//! New production code should import scope policy from
//! [`moatless_backend_core::scope`]. This module keeps the existing
//! `moatless_vibe_backend::scope::*` path valid for one extraction phase.

pub use moatless_backend_core::scope::*;
