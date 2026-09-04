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
    /// Stored as text. Convert via [`FeedbackCategory`]. `None` on a row
    /// written before the column existed, or whose submitter named none.
    pub category: Option<String>,
    pub comment: String,
    pub metadata: Option<serde_json::Value>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    /// Stored as text — `'open' | 'fixed' | 'not_relevant'`. Convert via
    /// [`FeedbackStatus`]. Written `'open'`; only a reader triaging the inbox
    /// moves it.
    pub status: String,
    /// Why the current status was set. Replaced with the status, so it always
    /// reads against the status beside it.
    pub status_note: Option<String>,
    /// User id of the reader who set the current status. `None` while the row
    /// is untriaged.
    pub status_updated_by: Option<String>,
    pub status_updated_at: Option<DateTime<Utc>>,
}

/// A feedback row joined to its task, for the cross-task listing. The task
/// columns are what a reader needs to place the comment; the feedback columns
/// are [`Feedback`]'s own.
#[derive(Debug, Clone)]
pub struct FeedbackWithTask {
    pub feedback: Feedback,
    /// `tasks.name`, falling back to the first line of `tasks.description`.
    pub task_title: String,
    pub repository_id: String,
    pub repository_name: Option<String>,
    pub workspace_id: Option<String>,
}

/// The task's most recent point of activity, used to scope feedback the
/// caller submitted without naming a message or tool call.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LatestActivity {
    pub message_uuid: String,
    pub tool_use_id: Option<String>,
}
