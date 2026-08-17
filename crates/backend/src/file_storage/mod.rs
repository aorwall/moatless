//! File storage — the HTTP surface here, the store below it in `moatless-core`.
//!
//! The handlers need `AuthUser` and the router needs `UserTokenVerifier`, both
//! from `moatless-identity`, so this half cannot sit beside the store.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade.

pub mod handlers;
pub mod routes;
pub mod schemas;

pub use moatless_core::file_storage::{dao, error, models, s3, store};
