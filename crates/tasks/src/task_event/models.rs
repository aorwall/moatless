//! Task event domain model.

use chrono::{DateTime, Utc};

/// A persisted task event row.
#[derive(Debug, sqlx::FromRow)]
pub struct TaskEvent {
    pub id: i64,
    pub event_id: String,
    pub task_id: String,
    pub event_type: String,
    pub sub_type: String,
    pub is_error: bool,
    pub agent_type: Option<String>,
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
    pub timestamp: String,
    pub created_at: DateTime<Utc>,
}

/// A task whose most recent turn is still open (`active` or `interrupted`) and
/// older than a cutoff — the reaper's stuck-turn backstop. Surfaced fleet-wide
/// by the `task_turns`-table query `TaskTurnDao::list_unclosed_active_turns`;
/// `task_turn::models` re-exports this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnclosedTurn {
    pub task_id: String,
    pub turn_number: Option<i32>,
    pub agent_type: Option<String>,
}
