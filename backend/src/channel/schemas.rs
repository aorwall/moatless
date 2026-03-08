//! Channel API request/response schemas.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::models::RoutingMode;
use super::provider::ChannelType;
use crate::scope::Scope;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelConfigRequest {
    pub provider: ChannelType,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
    pub response_config_id: Option<String>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelConfigRequest {
    pub source_name: Option<String>,
    pub repository_id: Option<String>,
    pub routing_mode: Option<RoutingMode>,
    pub prompt_template: Option<Option<String>>,
    pub active: Option<bool>,
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
    pub response_config_id: Option<Option<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
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
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub response_config_id: Option<String>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChannelConfigsQuery {
    pub provider: Option<String>,
    pub repository_id: Option<String>,
}

/// Available Slack channel info for dropdown selection.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlackChannelInfoResponse {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingQuery {
    pub config_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprovePairingRequest {
    pub config_id: String,
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PairingRequestResponse {
    pub id: i32,
    pub config_id: String,
    pub provider_user_id: String,
    pub code: String,
    pub status: String,
    #[schema(value_type = Option<Object>)]
    pub meta: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

impl PairingRequestResponse {
    pub fn from_model(r: &super::telegram::pairing::PairingRequest) -> Self {
        Self {
            id: r.id,
            config_id: r.config_id.clone(),
            provider_user_id: r.provider_user_id.clone(),
            code: r.code.clone(),
            status: r.status.clone(),
            meta: r.meta.clone(),
            created_at: r.created_at.to_rfc3339(),
            updated_at: r.updated_at.to_rfc3339(),
        }
    }
}

impl ChannelConfigResponse {
    pub fn from_model(row: &super::models::ChannelConfig) -> Self {
        let provider = ChannelType::parse(&row.provider)
            .unwrap_or(ChannelType::Slack);
        Self {
            id: row.id.clone(),
            provider,
            source_id: row.source_id.clone(),
            source_name: row.source_name.clone(),
            repository_id: row.repository_id.clone(),
            routing_mode: row.routing_mode,
            prompt_template: row.prompt_template.clone(),
            active: row.active,
            provider_config: row.provider_config.clone(),
            ongoing_task_id: row.ongoing_task_id.clone(),
            response_config_id: row.response_config_id.clone(),
            scope: row.scope,
            team_id: row.team_id.clone(),
            created_by: row.created_by.clone(),
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
        }
    }
}
