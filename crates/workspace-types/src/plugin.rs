//! Plugin contents selected for a sandbox.

use serde::{Deserialize, Serialize};

crate::string_enum!(
    /// Who declared a Plugin.
    PluginSource {
        /// Registered through the admin API, which may delete it.
        Manual => "manual",
        /// Declared by the deployment's `PLUGIN_REPOSITORIES_JSON` and
        /// reconciled on every start. Read-only to the admin API.
        Deployment => "deployment",
    }
);

/// Repo-relative skill roots, in precedence order; both are scanned and merged,
/// so `.agents/skills` (Codex) is found whether or not `.claude/skills` (Claude)
/// symlinks to it.
pub const REPO_SKILL_ROOTS: [&str; 2] = [".claude/skills", ".agents/skills"];

/// Plugin manifest paths, in precedence order. `.codex-plugin` is the Codex
/// repo-local plugin format; `.claude-plugin` remains supported for existing
/// Claude Code plugins.
pub const PLUGIN_MANIFEST_PATHS: [&str; 3] = [
    ".codex-plugin/plugin.json",
    ".agents-plugin/plugin.json",
    ".claude-plugin/plugin.json",
];

/// Plugin marketplace paths, in precedence order.
pub const PLUGIN_MARKETPLACE_PATHS: [&str; 2] = [
    ".agents/plugins/marketplace.json",
    ".claude-plugin/marketplace.json",
];

/// One plugin as handed to a sandbox: where to get it, and which of its skills
/// were picked.
///
/// The sandbox fetches the plugin from the backend (`FetchPlugin`, by `name`)
/// and removes the skill directories not named here, keeping everything else.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_source_not_starting_with_dot_slash_is_skipped() {
        let source = PluginMarketplaceSource::LocalPath("plugins/skills".to_string());
        assert_eq!(marketplace_local_path(&source, "ctx"), None);
    }

    #[test]
    fn a_local_descriptor_without_a_path_is_skipped() {
        let source = PluginMarketplaceSource::Descriptor {
            source: "local".to_string(),
            path: None,
        };
        assert_eq!(marketplace_local_path(&source, "ctx"), None);
    }

    #[test]
    fn a_non_local_descriptor_is_skipped() {
        let source = PluginMarketplaceSource::Descriptor {
            source: "git".to_string(),
            path: Some("./plugins/skills".to_string()),
        };
        assert_eq!(marketplace_local_path(&source, "ctx"), None);
    }

    #[test]
    fn a_string_source_starting_with_dot_slash_resolves() {
        let source = PluginMarketplaceSource::LocalPath("./plugins/skills".to_string());
        assert_eq!(
            marketplace_local_path(&source, "ctx"),
            Some("./plugins/skills".to_string())
        );
    }

    #[test]
    fn a_local_descriptor_with_a_path_resolves() {
        let source = PluginMarketplaceSource::Descriptor {
            source: "local".to_string(),
            path: Some("./plugins/skills".to_string()),
        };
        assert_eq!(
            marketplace_local_path(&source, "ctx"),
            Some("./plugins/skills".to_string())
        );
    }
}
