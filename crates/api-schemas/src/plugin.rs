//! Plugin request/response types.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

/// Who an activation applies to: everyone, one person, or one Workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum ActivationReach {
    /// The default position for everybody. Administrators only.
    Everyone,
    /// One person's own setting, which overrides the default. Which person is
    /// carried alongside as `userId`, and is the caller when that is absent.
    Personal,
    /// One Workspace's setting, which overrides both the default and a
    /// person's own — so a Workspace can add a skill nobody turned on and
    /// keep out one that everybody has. Which Workspace is carried alongside
    /// as `workspaceId`, and is required.
    Workspace,
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
    /// Whose record, with `personal` reach: the caller's own when omitted, and
    /// somebody else's only for an administrator — which is how a bot user,
    /// who never signs in to set its own, gets one. Rejected with `everyone`,
    /// where naming a person would read as a record for them and write the
    /// deployment-wide default instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    /// Which Workspace's record, with `workspace` reach. Required there and
    /// rejected with any other reach, where it would read as a record scoped
    /// to that Workspace and write a broader one instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
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
    /// Whose record this is. Absent for the deployment-wide default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    /// Which Workspace's record this is. Absent for every other reach.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
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

/// One plugin as handed to a sandbox: where to get it, and which of its skills
/// were picked.
///
/// This is decision 2 on the wire — the sandbox clones the plugin and removes
/// the skill directories not named here, keeping everything else the plugin
/// contains so its manifest, commands, hooks and agents still work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PluginDelivery {
    pub name: String,
    pub git_url: String,
    /// Never empty — a plugin with no picked skills is not sent at all.
    pub skill_names: Vec<String>,
}
