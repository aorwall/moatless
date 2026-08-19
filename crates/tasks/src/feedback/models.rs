//! Feedback domain types — DB row + shared enum re-exports.

use chrono::{DateTime, Utc};

/// One feedback row from the `feedback` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Feedback {
    pub feedback_id: String,
    pub task_id: String,
    pub message_uuid: Option<String>,
    pub tool_use_id: Option<String>,
    /// Stored as text — `'user' | 'agent'`. Convert via [`FeedbackSource`].
    pub source: String,
    /// Stored as text — `'task' | 'message' | 'tool_call' | 'preview_failure'`.
    /// Convert via [`FeedbackSurface`].
    pub surface: String,
    pub comment: String,
    pub metadata: Option<serde_json::Value>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}
