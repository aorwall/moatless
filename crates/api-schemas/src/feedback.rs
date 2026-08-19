//! Feedback request/response types.

use serde::{Deserialize, Serialize};

/// Submitter of a feedback row — derived from auth context on the server.
pub use moatless_types::feedback::FeedbackSource;

/// Where in the UI/CLI the feedback originated.
pub use moatless_types::feedback::FeedbackSurface;

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
