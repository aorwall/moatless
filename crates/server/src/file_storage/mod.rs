//! File storage — the HTTP surface over `moatless_core::file_storage`.
//!
//! The handlers need `AuthUser` and the router needs `UserTokenVerifier`, both
//! from `moatless-identity`, so this half cannot sit beside the operations in
//! `moatless-core`.

pub mod handlers;
pub mod routes;
pub mod schemas;
