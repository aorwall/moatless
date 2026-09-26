//! What the trajectory records and answers: sessions, events, cuts and their
//! errors.

use moatless_types::agent::Harness;
use serde_json::Value;
use uuid::Uuid;

/// Lines to append to one harness session, `first_seq` onward.
#[derive(Debug, Clone, Copy)]
pub struct AppendRequest<'a> {
    pub task_id: &'a str,
    pub harness: Harness,
    pub harness_session_id: &'a str,
    pub first_seq: i64,
    pub lines: &'a [NewEvent],
    pub turn_number: Option<i32>,
    /// Read only when the append creates the session.
    pub subagent_parent: Option<SubagentParent<'a>>,
}

/// The tool call in another session of the same Task that spawned a
/// subagent session.
#[derive(Debug, Clone, Copy)]
pub struct SubagentParent<'a> {
    pub harness_session_id: &'a str,
    pub tool_use_id: &'a str,
}

/// One harness session line to record.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub kind: String,
    pub harness_item_id: Option<String>,
    pub harness_turn_id: Option<String>,
    /// The line verbatim, without its newline.
    pub payload: String,
    /// `{role, blocks}`, as the sandbox's `SessionFormat` derived it.
    pub content: Value,
}

/// The reply to an append: `accepted` is false when the batch starts past
/// `last_seq + 1`, and the sender resends from `last_seq + 1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendOutcome {
    pub accepted: bool,
    pub last_seq: i64,
    /// The payload at `last_seq`, read only for an append with no lines.
    pub last_payload: Option<String>,
}

/// A `trajectory_sessions` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TrajectorySession {
    pub id: Uuid,
    pub task_id: String,
    pub harness: String,
    pub harness_session_id: String,
    pub forked_from_event_id: Option<Uuid>,
    /// A subagent session's spawning tool-call event.
    pub parent_event_id: Option<Uuid>,
    pub last_event_id: Option<Uuid>,
    pub last_seq: i64,
}

/// Where a fork cuts its source session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkCut {
    /// Claude: through the last line of this `message.id` (or line `uuid`).
    Message(String),
    /// Codex: through this turn's `task_complete` line.
    Turn(String),
    /// Neither anchor: the source's last line.
    End,
}

#[derive(Debug, thiserror::Error)]
pub enum AppendError {
    #[error("session {0} belongs to another task or harness")]
    Foreign(String),
    /// The spawning tool call is not recorded yet; the sender retries.
    #[error("subagent session {0}: its spawning tool call is not recorded")]
    ParentMissing(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    /// The walk from the session's last event did not reach seq 0 through
    /// every seq in between.
    #[error("trajectory chain is missing seq {expected}")]
    Gap { expected: i64 },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}
