//! Channel API request/response schemas.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelConfigRequest {
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: String,
    pub prompt_template: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelConfigRequest {
    pub source_name: Option<String>,
    pub routing_mode: Option<String>,
    pub prompt_template: Option<Option<String>>,
    pub active: Option<bool>,
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfigResponse {
    pub id: String,
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: String,
    pub prompt_template: Option<String>,
    pub active: bool,
    #[schema(value_type = Option<Object>)]
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
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

impl ChannelConfigResponse {
    pub fn from_model(row: &super::models::ChannelConfig) -> Self {
        Self {
            id: row.id.clone(),
            provider: row.provider.clone(),
            source_id: row.source_id.clone(),
            source_name: row.source_name.clone(),
            repository_id: row.repository_id.clone(),
            routing_mode: row.routing_mode.clone(),
            prompt_template: row.prompt_template.clone(),
            active: row.active,
            provider_config: row.provider_config.clone(),
            ongoing_task_id: row.ongoing_task_id.clone(),
            created_by: row.created_by.clone(),
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
        }
    }
}
