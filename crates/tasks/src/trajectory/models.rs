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
    /// A switch session's first seq its CLI wrote; read only while the
    /// session has no event.
    pub own_from_seq: Option<i64>,
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
    /// What projecting the appended lines changed.
    pub projected: LineProjection,
}

/// What projecting appended lines changed, for the live notices a commit owes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineProjection {
    /// Top-level Messages a reader sees change, by uuid, first change first.
    pub messages: Vec<String>,
    /// Subagent transcripts that changed, by spawning tool-use id.
    pub subagents: Vec<String>,
    /// The tool calls the lines settled: tool name (`None` for a call never
    /// recorded) and whether it failed.
    pub settled: Vec<(Option<String>, bool)>,
    /// Skills the lines' `Skill` calls loaded, in call order.
    pub skills: Vec<String>,
}

impl LineProjection {
    pub(crate) fn touch_message(&mut self, uuid: &str) {
        if !self.messages.iter().any(|m| m == uuid) {
            self.messages.push(uuid.to_string());
        }
    }

    pub(crate) fn touch_subagent(&mut self, tool_use_id: &str) {
        if !self.subagents.iter().any(|m| m == tool_use_id) {
            self.subagents.push(tool_use_id.to_string());
        }
    }
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
    /// A switch session's other-harness tip its prefix was rendered from.
    pub transcoded_from_event_id: Option<Uuid>,
    /// A switch session's first seq its CLI wrote.
    pub own_from_seq: Option<i64>,
    pub last_event_id: Option<Uuid>,
    pub last_seq: i64,
}

/// The event a Task continues from: its latest recorded top-level session's
/// tip, or a fork's cut before the fork records anything.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Head {
    pub event_id: Uuid,
    pub harness: String,
    pub harness_session_id: String,
    /// The head is the fork cut, in a session of the source Task.
    pub forked: bool,
}

/// Where a fork or rewind cuts a Task, and the harness of the session it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    pub event_id: Uuid,
    pub harness: String,
}

/// One line of another harness a switch session renders into its prefix.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscodeItem {
    /// The line's `content`, as the sandbox classified it.
    pub content: Value,
    pub turn_number: Option<i32>,
    pub source_harness: String,
}

/// A session to write out, and the items to render after its lines.
#[derive(Debug, Clone)]
pub struct OpenedSession {
    pub session: TrajectorySession,
    pub transcode: Vec<TranscodeItem>,
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
    /// A switch session's first append carries no `own_from_seq`.
    #[error("switch session {0}: its first append carries no own_from_seq")]
    OwnFromSeqMissing(String),
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
