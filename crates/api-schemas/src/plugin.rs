//! Plugin request/response types.

use serde::{Deserialize, Serialize};

/// Request to register a new plugin.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreatePluginRequest {
    pub name: String,
    pub git_url: String,
}

/// Plugin response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PluginResponse {
    pub id: String,
    pub name: String,
    pub git_url: String,
    /// What the source called its current state at the last successful sync.
    /// Absent until a first sync succeeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synced_version: Option<String>,
}

/// One skill inside a plugin, as stored at the plugin's synced version.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PluginSkillResponse {
    pub name: String,
    pub description: String,
}
