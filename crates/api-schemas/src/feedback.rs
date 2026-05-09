//! Feedback request/response types.

use serde::{Deserialize, Serialize};

/// Submitter of a feedback row — derived from auth context on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSource {
    /// Submitted by a regular user session.
    User,
    /// Submitted by the agent via the sandbox-issued user token.
    Agent,
}

impl FeedbackSource {
    pub fn as_str(&self) -> &'static str {
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
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            other => Err(format!("unknown FeedbackSource: {other:?}")),
        }
    }
}

/// Where in the UI/CLI the feedback originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSurface {
    Task,
    Message,
    ToolCall,
    PreviewFailure,
}

impl FeedbackSurface {
    pub fn as_str(&self) -> &'static str {
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
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "task" => Ok(Self::Task),
            "message" => Ok(Self::Message),
            "tool_call" => Ok(Self::ToolCall),
            "preview_failure" => Ok(Self::PreviewFailure),
            other => Err(format!("unknown FeedbackSurface: {other:?}")),
        }
    }
}

/// Request body for `POST /api/v1/feedback`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateFeedbackRequest {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    pub surface: FeedbackSurface,
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Response shape for a single feedback row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FeedbackResponse {
    pub feedback_id: String,
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    pub source: FeedbackSource,
    pub surface: FeedbackSurface,
    pub comment: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub created_by: String,
    pub created_at: String,
}

/// Response wrapper for `GET /api/v1/tasks/{task_id}/feedback`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListFeedbackResponse {
    pub items: Vec<FeedbackResponse>,
}
