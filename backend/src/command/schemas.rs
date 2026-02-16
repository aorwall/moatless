//! Command request/response types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::shared_models::AgentMode;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateCommandRequest {
    pub name: String,
    pub description: String,
    pub prompt: String,
    #[schema(value_type = Option<Object>)]
    pub conditions: Option<serde_json::Value>,
    pub icon: Option<String>,
    pub variant: Option<String>,
    pub active: Option<bool>,
    pub will_not_resume: Option<bool>,
    pub agent_mode: Option<AgentMode>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCommandRequest {
    pub description: Option<String>,
    pub prompt: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub conditions: Option<serde_json::Value>,
    pub icon: Option<String>,
    pub variant: Option<String>,
    pub active: Option<bool>,
    pub will_not_resume: Option<bool>,
    pub agent_mode: Option<AgentMode>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandResponse {
    pub name: String,
    pub description: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
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

impl From<super::models::Command> for CommandResponse {
    fn from(row: super::models::Command) -> Self {
        Self {
            name: row.name,
            description: row.description,
            prompt: row.prompt,
            conditions: row.conditions,
            icon: row.icon,
            variant: row.variant,
            active: row.active,
            will_not_resume: row.will_not_resume,
            agent_mode: row.agent_mode,
        }
    }
}
