//! The two durable change feeds, and the lock order between them.
//!
//! A feed is a counter and a log. Every mutation that a reader must learn about
//! allocates a position from the counter **in that mutation's own transaction**
//! and writes a record at it. That placement is the whole design, and it is why
//! these live here rather than with either mutator: the writers span bands —
//! `moatless-tasks` writes most of them, `moatless-sandbox-control` writes one
//! when Sandbox intent moves — and a feed whose record commits apart from the
//! mutation it describes is a feed that lies.
//!
//! # Why the counter, and not a `bigserial`
//!
//! An id assigned at INSERT and made visible at COMMIT can be read out of
//! order: two writers take 100 and 101, 101 commits first, and a reader polling
//! `> 99` advances past 100 and never sees it — silently, while reporting
//! itself up to date. Allocating from a counter row and holding it until commit
//! makes commit order and position order the same order.
//!
//! # The lock order
//!
//! Holding a counter row until commit is what costs callers something, because
//! two transactions that take both counters in opposite orders deadlock. There
//! is therefore one order, and it is not negotiable:
//!
//! 1. every other lock the transaction needs — the `tasks` or `sandboxes` row it
//!    is actually mutating;
//! 2. [`conversation`] — one row per Task, so contention is per-conversation;
//! 3. [`listing`] — one row for the whole deployment, so it is the contended
//!    one and is held for the shortest possible time.
//!
//! Taking the listing counter last makes it the innermost lock of everything
//! that touches it: a total order by construction, needing no agreement between
//! callers. Each feed's own docs restate the half of this that its callers must
//! obey; this is the statement they are restating.
//!
//! # What is not here
//!
//! Pruning. Both feeds are bounded by a background worker that deletes what has
//! fallen out of the retention window, and those workers are timers registered
//! with the root `Supervisor` — a scheduling concern in the band that owns
//! supervision, not part of the primitive. They live beside the domain they
//! prune and call the `prune_changes` in each module here.

pub mod conversation;
pub mod listing;
