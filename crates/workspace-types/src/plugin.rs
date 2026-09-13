//! Plugin contents selected for a sandbox.

use serde::{Deserialize, Serialize};

/// One plugin as handed to a sandbox: where to get it, and which of its skills
/// were picked.
///
/// Decision 2 on the wire — the sandbox clones the plugin and removes skill
/// directories not named here, keeping everything else (manifest, commands,
/// hooks, agents) intact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PluginDelivery {
    pub name: String,
    pub git_url: String,
    /// Never empty — a plugin with no picked skills is not sent at all.
    pub skill_names: Vec<String>,
    /// The subset of `skillNames` to frontload into every fresh agent session,
    /// on top of whatever the turn itself asked for.
    ///
    /// Absent from a configuration written before the flag existed; `default`
    /// covers that as no frontloads until somebody says so.
    #[serde(default)]
    pub frontload_skill_names: Vec<String>,
}
