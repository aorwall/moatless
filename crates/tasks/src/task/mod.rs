//! Task CRUD module.

pub mod binding_dao;
pub mod binding_summary;
pub mod closer;
pub mod creation;
pub mod dao;
pub mod error;
pub mod handlers;
pub mod loop_reader;
pub mod models;
pub mod presenter;
pub mod reaper;
pub mod relations;
pub mod routes;
pub mod sandbox_lifecycle;
pub mod sandbox_waker;
pub mod schemas;
pub mod service;
pub mod statistics;

/// `db-tests` seeding shared with the sandbox-control band's lifecycle tests.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
