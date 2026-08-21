//! Task CRUD module.

pub mod closer;
pub mod creation;
pub mod dao;
pub mod error;
pub mod outbound_events;
pub mod handlers;
pub mod models;
pub mod presenter;
pub mod relations;
pub mod routes;
pub mod schemas;
pub mod service;
pub mod statistics;

/// `db-tests` seeding shared with the sandbox-control band's lifecycle tests.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
