//! Plugin domain types — DB row structs.

use chrono::{DateTime, Utc};

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
}

/// Row from the `plugin_activations` table.
///
/// `user_id: None` means everyone; `skill_name: None` means the whole plugin.
/// See [`crate::plugin::activation`] for how the two axes combine.
#[derive(sqlx::FromRow, Debug, Clone, PartialEq, Eq)]
pub struct ActivationRow {
    pub plugin_id: String,
    pub skill_name: Option<String>,
    pub user_id: Option<String>,
    pub enabled: bool,
}
