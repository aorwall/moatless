//! Task event domain model.

use chrono::{DateTime, Utc};

/// Insert parameters for a new task event.
pub struct InsertTaskEvent {
    pub event_id: String,
    pub task_id: String,
    pub event_type: String,
    pub sub_type: String,
    pub is_error: bool,
    pub agent_profile: Option<String>,
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
    pub timestamp: String,
}

/// A persisted task event row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskEvent {
    pub id: i64,
    pub event_id: String,
    pub task_id: String,
    pub event_type: String,
    pub sub_type: String,
    pub is_error: bool,
    pub agent_profile: Option<String>,
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
    pub timestamp: String,
    pub created_at: DateTime<Utc>,
}

/// A task whose turn is still open (`active`/`interrupted`) past a cutoff — the
/// reaper's stuck-turn backstop.
///
/// Surfaced by `TaskTurns::list_unclosed_active_turns`; re-exported from
/// `task_turn::models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnclosedTurn {
    pub task_id: String,
    pub turn_number: Option<i32>,
    pub agent_profile: Option<String>,
    /// When the turn became `interrupted`; `None` while it is `active`.
    pub interrupted_at: Option<DateTime<Utc>>,
}
