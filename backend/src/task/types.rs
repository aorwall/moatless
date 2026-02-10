//! Task request/response types.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    pub repository_id: String,
    pub agent_type: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub branch: Option<String>,
    pub message: Option<String>,
    pub model: Option<String>,
    pub agent_mode: Option<String>,
    #[serde(default)]
    pub contexts: Vec<MessageContextInput>,
    #[serde(default)]
    pub file_ids: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateVibeTaskRequest {
    pub template_id: String,
    pub message: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct MessageContextInput {
    #[serde(rename = "type")]
    pub context_type: String,
    pub reference_id: String,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskMetadataRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVisibilityRequest {
    pub visibility: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetTagsRequest {
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub status: Option<String>,
    pub repository_id: Option<String>,
    pub created_by: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskResponse {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub agent_type: String,
    pub repository_id: String,
    pub repository_name: Option<String>,
    pub branch: Option<String>,
    pub created_at: String,
    pub last_message_at: String,
    pub total_cost_usd: f64,
    pub site_url: Option<String>,
    pub template_id: Option<String>,
    pub vibe: Option<bool>,
    pub created_by: Option<String>,
    pub created_by_display_name: Option<String>,
    pub visibility: String,
    pub tags: Vec<TagResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_running: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagResponse {
    pub tag_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub status: String,
    pub task: TaskResponse,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResponse {
    pub items: Vec<TaskResponse>,
    pub pagination: PaginationResponse,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginationResponse {
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
    pub has_more: bool,
}
