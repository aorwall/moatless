//! Command request/response types.

use serde::{Deserialize, Serialize};

use crate::shared::AgentMode;

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateCommandRequest {
    pub name: String,
    pub description: String,
    pub prompt: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub conditions: Option<serde_json::Value>,
    pub icon: Option<String>,
    pub variant: Option<String>,
    pub active: Option<bool>,
    pub will_not_resume: Option<bool>,
    pub agent_mode: Option<AgentMode>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateCommandRequest {
    pub description: Option<String>,
    pub prompt: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub conditions: Option<serde_json::Value>,
    pub icon: Option<String>,
    pub variant: Option<String>,
    pub active: Option<bool>,
    pub will_not_resume: Option<bool>,
    pub agent_mode: Option<AgentMode>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CommandResponse {
    pub name: String,
    pub description: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub conditions: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub active: Option<bool>,
    pub will_not_resume: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<AgentMode>,
}
