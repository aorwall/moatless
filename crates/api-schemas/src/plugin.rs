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

/// Who an activation applies to: everyone, or the calling user alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum ActivationReach {
    /// The default position for everybody. Administrators only.
    Everyone,
    /// The calling user's own setting, which overrides the default.
    Personal,
}

/// Turn a plugin, or one skill inside it, on or off.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetActivationRequest {
    pub reach: ActivationReach,
    /// Omit to mean the whole plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    pub enabled: bool,
}

/// One stored activation record.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ActivationResponse {
    pub plugin_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    pub reach: ActivationReach,
    pub enabled: bool,
}

/// What the calling user actually gets, after resolution.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct EffectivePluginResponse {
    pub plugin_id: String,
    pub plugin_name: String,
    /// Never empty — a plugin with no picked skills is not delivered at all.
    pub skill_names: Vec<String>,
}
