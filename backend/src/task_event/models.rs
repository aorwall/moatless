//! Task event domain model.

use chrono::{DateTime, Utc};

/// A persisted task event row.
#[derive(Debug, sqlx::FromRow)]
pub struct TaskEvent {
    pub id: i32,
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
