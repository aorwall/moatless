//! Task template request/response types.

use serde::{Deserialize, Serialize};

use crate::enums::Scope;

/// Request to create a task template.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskTemplateRequest {
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub repository_id: String,
    pub agent_type: String,
    pub agent_mode: Option<String>,
    pub default_model: Option<String>,
    pub initial_prompt: Option<String>,
    pub skills: Option<Vec<String>>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
    pub featured: Option<bool>,
    pub display_order: Option<i32>,
}

/// Request to update a task template (all fields optional).
#[derive(Debug, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskTemplateRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub repository_id: Option<String>,
    pub agent_type: Option<String>,
    pub agent_mode: Option<String>,
    pub default_model: Option<String>,
    pub initial_prompt: Option<String>,
    pub skills: Option<Vec<String>>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
    pub featured: Option<bool>,
    pub display_order: Option<i32>,
}

/// Task template response DTO.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskTemplateResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub repository_id: String,
    pub agent_type: String,
    pub agent_mode: Option<String>,
    pub default_model: Option<String>,
    pub initial_prompt: Option<String>,
    pub skills: Option<Vec<String>>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub featured: bool,
    pub display_order: i32,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
