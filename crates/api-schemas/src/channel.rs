//! Channel API request/response schemas.

use serde::{Deserialize, Serialize};

use crate::enums::{ChannelType, RoutingMode, Scope};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelConfigRequest {
    pub provider: ChannelType,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub response_config_id: Option<String>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Option<Vec<String>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelConfigRequest {
    pub source_name: Option<String>,
    pub repository_id: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    pub prompt_template: Option<Option<String>>,
    pub active: Option<bool>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub response_config_id: Option<Option<String>>,
    pub task_name: Option<Option<String>>,
    pub tag_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfigResponse {
    pub id: String,
    pub provider: ChannelType,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    pub active: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub response_config_id: Option<String>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_by: String,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Request to send a message through channel(s) associated with a task.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageRequest {
    pub text: String,
}

/// Response from sending a message through channel(s).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageResponse {
    pub sent_count: u32,
    pub errors: Vec<String>,
}

/// Channel connected to a task (via channel_threads).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskChannelResponse {
    pub config_id: String,
    pub provider: ChannelType,
    pub source_id: String,
    pub source_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetChannelDefaultsRequest {
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub provider_config: Option<serde_json::Value>,
    pub response_config_id: Option<String>,
    pub team_id: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDefaultsQuery {
    pub team_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChannelConfigsQuery {
    pub provider: Option<String>,
    pub repository_id: Option<String>,
}

/// Available Slack channel info for dropdown selection.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SlackChannelInfoResponse {
    pub id: String,
    pub name: String,
}

/// Request body for manually triggering a channel config.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerChannelRequest {
    /// Optional message override. If omitted, uses the channel's configured message.
    pub message: Option<String>,
}

/// Response from manually triggering a channel config.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TriggerChannelResponse {
    pub task_id: String,
    pub short_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingQuery {
    pub config_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ApprovePairingRequest {
    pub config_id: String,
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PairingRequestResponse {
    pub id: i32,
    pub config_id: String,
    pub provider_user_id: String,
    pub code: String,
    pub status: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub meta: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}
