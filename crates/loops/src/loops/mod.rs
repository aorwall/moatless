//! Loop domain module: the standing definition of recurring agent work
//! (Schedule or Subscription firing conditions, spawn config, and the runtime
//! paths that fire it).
//!
//! Owns the whole domain: [`operations`] over the crate's private Loop store,
//! [`firing`], [`sync`] (reconciling `.moatless/loops/<name>/LOOP.md`),
//! [`http`], and [`ports`] (the Subscription reads Adapters make).

pub(crate) mod check_run_trigger;
pub(crate) mod dao;
pub mod firing;
pub mod http;
pub mod models;
pub mod ports;
pub mod sync;

pub(crate) mod persistence;

pub(crate) mod access;
pub mod schedule;

pub mod operations;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
