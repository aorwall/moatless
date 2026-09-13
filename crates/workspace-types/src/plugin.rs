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

/// One entry's `source` value from a repo's on-disk `marketplace.json`.
///
/// Every reader parses this independently off the raw file (a sandbox
/// resolving local plugin directories, and workspaces fetching a plugin's
/// skills over the GitHub API without a clone), so this is only the shared
/// wire shape — not a single parse path.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum PluginMarketplaceSource {
    LocalPath(String),
    Descriptor {
        source: String,
        #[serde(default)]
        path: Option<String>,
    },
}

/// A marketplace entry's local source path, or `None` for a non-local source.
///
/// `context` is logged as-is on the skip path; callers pass whatever
/// identifies the entry for their own diagnostics (a plugin name, or a git
/// URL where no plugin name is in scope yet).
pub fn marketplace_local_path(source: &PluginMarketplaceSource, context: &str) -> Option<String> {
    match source {
        PluginMarketplaceSource::LocalPath(path) => {
            if path.trim().starts_with("./") {
                Some(path.clone())
            } else {
                tracing::debug!(
                    context,
                    source = %path,
                    "Skipping non-local plugin marketplace string source"
                );
                None
            }
        }
        PluginMarketplaceSource::Descriptor { source, path } if source == "local" => path.clone(),
        PluginMarketplaceSource::Descriptor { source, .. } => {
            tracing::debug!(
                context,
                source = %source,
                "Skipping non-local plugin marketplace descriptor source"
            );
            None
        }
    }
}
