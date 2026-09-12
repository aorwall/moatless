//! Message module — retrieval, sending, transformation.

pub mod contexts;
pub(crate) mod dao;
pub mod enricher;
pub mod error;
pub mod handlers;
pub mod hooks;
pub mod models;
pub mod operations;
pub mod routes;
pub mod schemas;
pub mod send;
pub mod service;
pub(crate) mod summary;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod transform;
pub mod wake;

pub(crate) mod fork_dao;

mod external_events_dao;
mod recovery_dao;
