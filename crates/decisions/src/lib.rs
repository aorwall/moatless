//! Multiple-choice decisions answered by a backend-proxied decision provider.
//!
//! The provider's credentials stay in the backend: a sandbox reaches the
//! provider only through `POST /api/v1/decisions/pick`.

pub mod compose;
pub mod metrics;
pub mod routes;

mod error;
mod handlers;
mod jev;
mod operations;
mod provider;
mod state;

#[cfg(test)]
#[expect(clippy::expect_used, reason = "a broken fixture fails its test loudly")]
mod tests;
