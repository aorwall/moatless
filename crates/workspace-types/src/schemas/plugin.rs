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
    /// Path of the `SKILL.md` inside the plugin repository. Same contract as
    /// `SkillResponse::path`, which states what a reader does with it.
    pub path: String,
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
    /// One Workspace's setting, which overrides both the default and a person's
    /// own — a Workspace can add a skill nobody turned on, or keep out one
    /// everybody has.
    ///
    /// Carried alongside as `workspaceId`, which is required.
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
    /// Whose record, with `personal` reach: the caller's own when omitted, or
    /// (admin only) somebody else's — how a bot user gets one set.
    ///
    /// Rejected with `everyone`, where naming a person would misleadingly write
    /// the deployment-wide default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    /// Which Workspace's record, with `workspace` reach. Required there and
    /// rejected with any other reach, where it would read as a record scoped to
    /// that Workspace and write a broader one instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub enabled: bool,
    /// Lift this record above every other record regardless of reach, so it
    /// keeps applying even in a Workspace that overrides or disables it.
    ///
    /// Omit to leave the existing value — a control that only knows on/off
    /// cannot clear it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always: Option<bool>,
    /// Frontload the skill's body into every fresh agent session it reaches,
    /// rather than waiting to be picked for a turn. Omitted the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontload: Option<bool>,
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
    /// Whether this record outranks every record that is not `always`.
    pub always: bool,
    /// Whether the skills this record decides frontload into every fresh agent
    /// session.
    pub frontload: bool,
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
    /// The subset of `skillNames` that frontloads into every fresh agent
    /// session rather than waiting to be picked for a turn.
    #[serde(default)]
    pub frontload_skill_names: Vec<String>,
}

pub use crate::plugin::PluginDelivery;
