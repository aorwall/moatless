//! The two durable change feeds, and the lock order between them.
//!
//! A feed is a counter and a log: every mutation a reader must learn about
//! allocates a position from the counter **in its own transaction** and writes
//! a record at it — why these live here rather than with either mutator, since
//! writers span bands.
//!
//! Holding the counter until commit (not a `bigserial`, which a reader can poll
//! past out of commit order) makes commit and position order the same. That
//! costs a fixed lock order: every other lock first, then [`conversation`] (one
//! row per Task), then [`listing`] last (one row for the whole deployment, so
//! it stays innermost).
//!
//! Pruning is not here — see each module's `prune_changes`, run by a background
//! worker.

pub mod conversation;
pub mod listing;
