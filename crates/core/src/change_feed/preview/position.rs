//! What a recorded open is, and how much of the feed is kept.
//!
//! No T3 vocabulary appears here — `moatless-t3code` turns the record back into
//! a preview event.

/// How many of the most recent opens are kept.
///
/// Nothing resumes from a position here — a reader's cursor starts at the head
/// on every boot — so this bounds table growth and nothing else.
pub const RETAINED_CHANGES: i64 = 5_000;

/// How many opens one poll takes. A reader further behind than this catches up
/// over the following polls rather than in one read.
pub const CHANGES_PER_POLL: i64 = 64;

/// One recorded open: the tab, where to point it, and where it sits relative to
/// every other.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PreviewTabOpen {
    pub position: i64,
    /// Identifies the store that already holds this tab, so its own reader can
    /// skip the record rather than announce the tab twice.
    pub origin: String,
    pub task_id: String,
    pub tab_id: String,
    pub url: String,
}
