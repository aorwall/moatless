//! Task CRUD module.

pub mod closer;
pub mod creation;
pub(crate) mod dao;
pub mod error;
pub mod fork;
pub mod handlers;
pub mod models;
pub mod operations;
pub mod presenter;
pub mod relations;
pub mod rewind;
pub(crate) mod rewind_dao;
pub mod routes;
pub mod schemas;
pub mod service;
pub mod statistics;

/// `db-tests` seeding shared with the sandbox-control band's lifecycle tests.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub mod bindings;

pub mod removal;
pub mod start;

pub(crate) mod outbound_events_dao;

pub(crate) mod fork_dao;

pub(crate) mod relations_dao;
