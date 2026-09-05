//! File storage — the HTTP surface and the store, over the two halves below.
//!
//! The handlers need `AuthUser` and the router needs `UserTokenVerifier`, both
//! from `moatless-identity`, so this half cannot sit beside the metadata in
//! `moatless-core`. `store` sits here because it is the one place that needs
//! both that metadata and `moatless-s3`.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade.

pub mod handlers;
pub mod routes;
pub mod schemas;
pub mod store;

pub use moatless_core::file_storage::{dao, error, models};
pub use moatless_s3 as s3;
