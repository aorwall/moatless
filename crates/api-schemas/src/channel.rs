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
    pub created_at: String,
    pub updated_at: String,
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
