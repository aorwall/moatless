//! What a recorded start is, and how much of the feed is kept.
//!
//! No T3 vocabulary appears here — `moatless-t3code` owns the shape of
//! [`TerminalSessionStart::summary`] and turns the record back into a terminal
//! the drawer lists.

use serde_json::Value;

/// How many of the most recent starts are kept.
///
/// Nothing resumes from a position here — a reader's cursor starts at the head
/// on every boot — so this bounds table growth and nothing else.
pub const RETAINED_CHANGES: i64 = 5_000;

/// How many starts one poll takes. A reader further behind than this catches up
/// over the following polls rather than in one read.
pub const CHANGES_PER_POLL: i64 = 64;

/// One recorded start: the session, everything needed to list it, and where it
/// sits relative to every other.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct TerminalSessionStart {
    pub position: i64,
    /// Identifies the hub that already holds this session, so its own reader can
    /// skip the record rather than raise the terminal twice.
    pub origin: String,
    pub task_id: String,
    pub terminal_id: String,
    /// The whole session as the reader will list it, carried rather than read
    /// back: the reader holds no Sandbox connection.
    pub summary: Value,
}
