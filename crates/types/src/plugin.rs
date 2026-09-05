//! Plugin contents selected for a sandbox.

use serde::{Deserialize, Serialize};

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
    /// The subset of `skillNames` to frontload into every fresh agent session,
    /// on top of whatever the turn itself asked for.
    ///
    /// Absent from a configuration written before the flag existed, which is
    /// what `default` covers: no skill frontloads until somebody says so.
    #[serde(default)]
    pub frontload_skill_names: Vec<String>,
}
