//! Scoped secret storage and resolution feature.

pub mod audit;
pub(crate) mod dao;
pub mod error;
pub mod handlers;
pub mod models;
pub mod routes;
pub mod schemas;
pub mod service;
// `test` stays first so the census keeps scoring this fake as test-context; the
// feature exposes it to other crates' tests (see Cargo.toml `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub(crate) mod audit_dao;
