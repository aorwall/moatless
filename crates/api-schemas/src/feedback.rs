//! Feedback request/response types.

use serde::{Deserialize, Serialize};

/// Submitter of a feedback row — derived from auth context on the server.
pub use moatless_types::feedback::FeedbackSource;

/// Where in the UI/CLI the feedback originated.
pub use moatless_types::feedback::FeedbackSurface;

/// What the note is about.
pub use moatless_types::feedback::FeedbackCategory;

/// What a reader decided about the note.
pub use moatless_types::feedback::FeedbackStatus;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<FeedbackCategory>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<FeedbackCategory>,
    pub comment: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub created_by: String,
    pub created_at: String,
    pub status: FeedbackStatus,
    /// Why the current status was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_note: Option<String>,
    /// User id of the reader who set the current status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_updated_by: Option<String>,
    /// RFC 3339 instant the status was last set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_updated_at: Option<String>,
}

/// Request body for `PATCH /api/v1/feedback/{feedback_id}/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateFeedbackStatusRequest {
    pub status: FeedbackStatus,
    /// Why. Replaces the note the previous status carried; an empty string
    /// reads as none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Response wrapper for `GET /api/v1/tasks/{task_id}/feedback`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListFeedbackResponse {
    pub items: Vec<FeedbackResponse>,
}

/// Query parameters for `GET /api/v1/feedback`.
///
/// Every filter is an AND; workspace/repository filters read the joined task,
/// not the feedback row, which carries only `task_id`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[serde(rename_all = "camelCase")]
pub struct ListAllFeedbackQuery {
    pub workspace_id: Option<String>,
    pub repository_id: Option<String>,
    pub task_id: Option<String>,
    pub source: Option<FeedbackSource>,
    pub surface: Option<FeedbackSurface>,
    pub category: Option<FeedbackCategory>,
    pub status: Option<FeedbackStatus>,
    /// User id of the submitter.
    pub created_by: Option<String>,
    /// RFC 3339 instant; keeps feedback created at or after it.
    pub since: Option<String>,
    /// Defaults to 50, capped at 200.
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// One row of `GET /api/v1/feedback`: the feedback flattened together with the
/// task context that makes a cross-task listing readable without a second call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FeedbackListItem {
    #[serde(flatten)]
    pub feedback: FeedbackResponse,
    /// `Task.name`, falling back to the first line of its description.
    pub task_title: String,
    /// `None` for a task on a Workspace that places no repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    /// `None` when the repository row was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

/// Response for `GET /api/v1/feedback`, newest first. `total` counts every row
/// matching the filters, not the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListAllFeedbackResponse {
    pub items: Vec<FeedbackListItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}
