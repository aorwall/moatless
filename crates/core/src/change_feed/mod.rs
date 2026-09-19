//! The durable change feeds, and the lock order between them.
//!
//! A feed is a counter and a log: a mutation readers must learn about takes a
//! position **in its own transaction** and writes its record there. The counter
//! is held until commit — not a `bigserial`, which a reader can poll past out
//! of commit order — so position order is commit order. That costs a fixed lock
//! order: every other lock first, then [`conversation`] (a row per Task), then
//! [`listing`] innermost (one row per deployment). [`events`] and [`preview`]
//! take that counter and no other, so they sit outside the order.

pub mod conversation;
pub mod events;
pub mod listing;
pub mod preview;
