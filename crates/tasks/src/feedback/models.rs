//! Feedback domain types — DB row + shared enum re-exports.

use chrono::{DateTime, Utc};

/// Submitter of a feedback row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSource {
    User,
    Agent,
}

impl FeedbackSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

impl std::fmt::Display for FeedbackSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for FeedbackSource {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            other => Err(format!("unknown FeedbackSource: {other:?}")),
        }
    }
}

/// Surface from which feedback originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSurface {
    Task,
    Message,
    ToolCall,
    PreviewFailure,
}

impl FeedbackSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Message => "message",
            Self::ToolCall => "tool_call",
            Self::PreviewFailure => "preview_failure",
        }
    }
}

impl std::fmt::Display for FeedbackSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for FeedbackSurface {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "task" => Ok(Self::Task),
            "message" => Ok(Self::Message),
            "tool_call" => Ok(Self::ToolCall),
            "preview_failure" => Ok(Self::PreviewFailure),
            other => Err(format!("unknown FeedbackSurface: {other:?}")),
        }
    }
}

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
