//! Loop domain module.
//!
//! A Loop is the standing definition of recurring agent work (CONTEXT.md,
//! the Loop model decision): its firing conditions (a Schedule or Subscription), the config
//! used to spawn tasks, and the runtime paths that fire it. This module owns
//! the whole domain:
//!
//! - [`dao`] — the single home for reads/writes of the four Loop tables.
//! - [`firing`] — schedule runner, inbound event handling, receipts, metrics.
//! - [`sync`] — reconciling `.moatless/loops/<name>/LOOP.md` from a repo's git
//!   tree into the Loop tables with explicit configuration authority.
//! - [`http`] — the `/api/v1/loops` HTTP surface.

pub mod dao;
pub mod firing;
pub mod http;
pub mod sync;
