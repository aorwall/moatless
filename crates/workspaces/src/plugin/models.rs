//! Plugin domain types — DB row structs.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Row from the `plugins` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub git_url: String,
    /// What the source called its current state at the last successful sync —
    /// for GitHub, the default branch's head commit SHA. `None` until a first
    /// sync succeeds.
    pub synced_version: Option<String>,
    pub synced_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Row from the `plugin_skills` table — one skill inside a plugin, as read out
/// of the plugin's content at the version recorded on its `Plugin`.
#[derive(sqlx::FromRow, Debug, Clone, PartialEq, Eq)]
pub struct PluginSkill {
    pub plugin_id: String,
    pub name: String,
    pub description: String,
    /// Plugin-relative path of the `SKILL.md`. Empty for a row written before
    /// the column existed, until the plugin next syncs.
    pub path: String,
}

/// Row from the `plugin_activations` table.
///
/// Names at most one subject — `user_id`, `workspace_id`, or neither (the
/// deployment default). `skill_name: None` means the whole plugin; see
/// [`crate::plugin::activation`] for how the axes combine.
#[derive(sqlx::FromRow, Debug, Clone, PartialEq, Eq)]
pub struct ActivationRow {
    pub plugin_id: String,
    pub skill_name: Option<String>,
    pub user_id: Option<Uuid>,
    pub workspace_id: Option<String>,
    pub enabled: bool,
    /// Outranks every record that is not `always`, whatever reach either of
    /// them carries.
    pub always: bool,
    /// Frontload the skill's body into every fresh agent session it reaches,
    /// rather than waiting to be picked for a turn.
    pub frontload: bool,
}

/// A write to one activation record.
///
/// The decision is stated outright; the two flags are `None` when unmentioned,
/// leaving whatever is stored alone — so a control that knows only on/off can
/// toggle a record without clearing flags it cannot see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivationWrite {
    pub plugin_id: String,
    pub skill_name: Option<String>,
    pub user_id: Option<Uuid>,
    pub workspace_id: Option<String>,
    pub enabled: bool,
    pub always: Option<bool>,
    pub frontload: Option<bool>,
}

/// One Workspace-reach record a `.moatless/workspaces.json` sync writes, with
/// `always` and `frontload` off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredActivation {
    pub plugin_id: String,
    /// `None` means the whole plugin.
    pub skill_name: Option<String>,
    pub enabled: bool,
}
