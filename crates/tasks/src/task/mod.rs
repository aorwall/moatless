//! Task CRUD module.

pub(crate) mod access_dao;
pub mod closer;
pub mod cooldown;
pub(crate) mod cooldown_dao;
pub mod creation;
pub(crate) mod dao;
pub mod error;
pub mod fork;
pub mod handlers;
mod merge_back;
pub(crate) mod merge_back_dao;
pub mod models;
pub mod operations;
pub(crate) mod pin_dao;
pub mod presenter;
pub mod relations;
pub mod repositories;
pub mod rewind;
pub(crate) mod rewind_dao;
pub mod routes;
pub mod sandbox_reach;
pub(crate) mod sandbox_reach_dao;
pub mod schemas;
pub mod service;
pub(crate) mod settle_dao;
pub(crate) mod side_chat_dao;
pub(crate) mod snooze_dao;
pub mod statistics;
pub mod switch;
pub(crate) mod switch_dao;

/// `db-tests` seeding shared with the sandbox-control band's lifecycle tests.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub mod bindings;

pub mod removal;
pub mod start;

pub(crate) mod outbound_events_dao;

pub(crate) mod fork_dao;

pub(crate) mod relations_dao;

pub(crate) mod repositories_dao;
