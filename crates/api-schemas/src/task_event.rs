//! Task event request/response types.

use serde::{Deserialize, Serialize};

/// Query parameters for listing task events.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[serde(rename_all = "camelCase")]
pub struct ListTaskEventsQuery {
    /// Comma-separated event type filter (e.g., `?types=sandbox,server,agent`).
    pub types: Option<String>,
    /// Maximum number of events to return (default 500, max 5000).
    pub limit: Option<i64>,
    /// Offset for pagination (default 0).
    pub offset: Option<i64>,
}

/// A single task event in the response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskEventResponse {
    pub event_id: String,
    pub task_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub sub_type: String,
    pub is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
    pub timestamp: String,
}

/// Response for listing task events.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListTaskEventsResponse {
    pub items: Vec<TaskEventResponse>,
}
