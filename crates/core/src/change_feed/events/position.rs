//! The durable record that a task event was published.
//!
//! One counter row for the whole deployment, held until commit. See the module
//! doc on [`super::super`] for why the hold is what makes commit order and
//! position order the same order.

/// How many of the most recent events are kept.
///
/// Nothing resumes from a position here — an SSE cursor starts at the head on
/// every boot — so this bounds table growth and nothing else. Wide enough that
/// a replica stalled by a rolling restart still finds its cursor's successor.
pub const RETAINED_CHANGES: i64 = 50_000;

/// How many events one dispatch claim takes.
pub const DISPATCH_BATCH: i64 = 64;

/// How long a claim stands before another replica may take the same events.
///
/// The floor is how long a batch of [`DISPATCH_BATCH`] handlers can legitimately
/// run: claim again below that and a slow batch is dispatched twice while the
/// first replica is still working it.
pub const RECLAIM_AFTER: std::time::Duration = std::time::Duration::from_mins(5);

/// One recorded event: the serialized envelope, and where it sits relative to
/// every other.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct EventChange {
    pub position: i64,
    pub event: String,
}
