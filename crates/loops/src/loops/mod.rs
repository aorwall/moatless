//! Loop domain module.
//!
//! A Loop is the standing definition of recurring agent work (CONTEXT.md,
//! the Loop model decision): its firing conditions (a Schedule or Subscription), the config
//! used to spawn tasks, and the runtime paths that fire it. This module owns
//! the whole domain:
//!
//! - [`operations`] — the Loop operations a host drives, over the crate's
//!   private store for the four Loop tables.
//! - [`firing`] — schedule runner, inbound event handling, receipts, metrics.
//! - [`sync`] — reconciling `.moatless/loops/<name>/LOOP.md` from a repo's git
//!   tree into the Loop tables with explicit configuration authority.
//! - [`http`] — the `/api/v1/loops` HTTP surface.
//! - [`ports`] — the Subscription reads the Adapters make.

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
