//! Stage plugin skills so that Codex can discover them.
//!
//! Claude Code consumes plugins via `PluginConfig { type: "local", path }`
//! entries in `ClaudeOptions`. Codex has no plugin system; it only discovers
//! skills via its admin skill path `/etc/codex/skills`. When a repo ships
//! with a plugin that bundles skills, we copy those skills into
//! `/etc/codex/skills/{plugin_name}--{skill_name}/` so the Codex agent picks
//! them up like any other skill.
//!
//! Runs after `clone_plugins()` during core init. Non-fatal: failures
//! are logged and skipped.
//!
//! Synchronous and recursive (it copies whole skill trees and removes stale
//! targets), so core init invokes it through [`super::run_init_blocking`]
//! rather than on a Tokio worker.

use std::path::{Path, PathBuf};

use moatless_types::plugin::PluginDelivery;
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::dirs::copy_dir_contents;

const PLUGINS_DIR: &str = "/opt/moatless/plugins";
const CODEX_SKILLS_DIR: &str = "/etc/codex/skills";

#[derive(Debug, Deserialize)]
struct PluginManifest {
    skills: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PluginMarketplace {
    #[serde(default)]
    plugins: Vec<PluginMarketplaceEntry>,
}

#[derive(Debug, Deserialize)]
struct PluginMarketplaceEntry {
    name: String,
    source: PluginMarketplaceSource,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginMarketplaceSource {
    LocalPath(String),
    Descriptor {
        source: String,
        #[serde(default)]
        path: Option<String>,
    },
}

const PLUGIN_MANIFEST_PATHS: [&str; 3] = [
    ".codex-plugin/plugin.json",
    ".agents-plugin/plugin.json",
    ".claude-plugin/plugin.json",
];

const PLUGIN_MARKETPLACE_PATHS: [&str; 2] = [
    ".agents/plugins/marketplace.json",
    ".claude-plugin/marketplace.json",
];

/// Scan `/opt/moatless/plugins/*`, resolve each plugin's `skills/` directory
/// (honoring plugin manifests and repo-local plugin marketplace files), and
/// copy every skill into `/etc/codex/skills/{plugin}--{skill}/`.
///
/// Returns the number of skills copied. Non-fatal — any error is logged and
/// the scan continues.
///
/// Blocking and recursive — call it through [`super::run_init_blocking`].
pub(super) fn stage_plugin_skills_for_codex() -> usize {
    stage_plugin_skills_with_roots(Path::new(PLUGINS_DIR), Path::new(CODEX_SKILLS_DIR))
}

fn stage_plugin_skills_with_roots(plugins_root: &Path, codex_skills_root: &Path) -> usize {
    if !plugins_root.is_dir() {
        return 0;
    }

    if let Err(e) = std::fs::create_dir_all(codex_skills_root) {
        warn!(
            path = %codex_skills_root.display(),
            error = %e,
            "Failed to create Codex skills directory — skipping plugin skill staging"
        );
        return 0;
    }

    let entries = match std::fs::read_dir(plugins_root) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                path = %plugins_root.display(),
                error = %e,
                "Failed to read plugins directory"
            );
            return 0;
        }
    };

    let mut total = 0usize;
    for entry in entries.flatten() {
        let plugin_path = entry.path();
        if !plugin_path.is_dir() {
            continue;
        }
        let plugin_name = match plugin_path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        total += stage_single_plugin(&plugin_path, &plugin_name, codex_skills_root);
    }

    if total > 0 {
        info!(
            count = total,
            target = %codex_skills_root.display(),
            "Staged plugin skills for Codex"
        );
    }
    total
}

fn stage_single_plugin(plugin_path: &Path, plugin_name: &str, codex_skills_root: &Path) -> usize {
    plugin_components(plugin_path, plugin_name)
        .iter()
        .map(|(name, root)| stage_plugin_component(root, name, codex_skills_root))
        .sum()
}

/// Remove every skill directory in a cloned plugin that is not in `picked`,
/// returning how many went.
///
/// This is where the activation records land: the backend resolved which skills
/// the task's owner gets, and everything else the plugin ships is deleted before
/// any agent sees it. The rest of the plugin — its manifest, commands, hooks and
/// agents — is untouched, so a plugin with one skill picked still works.
///
/// A skill is identified by its directory name, which is what the backend
/// stores. Non-fatal throughout: a directory we fail to remove stays, and the
/// agent sees one skill too many rather than the sandbox failing to start.
pub(super) fn keep_only(plugin_path: &Path, plugin_name: &str, picked: &[String]) -> usize {
    let mut dropped = 0usize;

    for (name, root) in plugin_components(plugin_path, plugin_name) {
        let skills_dir = root.join(resolve_plugin_skills_subdir(&root));
        let entries = match std::fs::read_dir(&skills_dir) {
            Ok(entries) => entries,
            // No skills directory at all is the ordinary case for a plugin that
            // ships only commands or hooks.
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(skill) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if picked.iter().any(|p| p == skill) {
                continue;
            }
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    debug!(plugin = %name, skill = %skill, "Removed unpicked skill");
                    dropped += 1;
                }
                Err(e) => {
                    warn!(
                        plugin = %name,
                        skill = %skill,
                        error = %e,
                        "Failed to remove unpicked skill (non-fatal)"
                    );
                }
            }
        }
    }

    dropped
}

/// The `(name, root)` pairs a cloned plugin directory contains. A repo carrying
/// a plugin marketplace with local entries contributes one per entry; anything
/// else is a single component at the root.
fn plugin_components(plugin_path: &Path, plugin_name: &str) -> Vec<(String, PathBuf)> {
    match marketplace_components(plugin_path) {
        Some(components) => components,
        None => vec![(plugin_name.to_string(), plugin_path.to_path_buf())],
    }
}

fn marketplace_components(plugin_path: &Path) -> Option<Vec<(String, PathBuf)>> {
    let (marketplace_path, raw) = read_first_marketplace(plugin_path)?;
    let marketplace: PluginMarketplace = match serde_json::from_str(&raw) {
        Ok(marketplace) => marketplace,
        Err(e) => {
            warn!(
                marketplace = %marketplace_path.display(),
                error = %e,
                "Invalid .agents plugin marketplace — falling back to root plugin"
            );
            return None;
        }
    };

    let mut components = Vec::new();
    for plugin in marketplace.plugins {
        let Some(source_path) = marketplace_local_path(&plugin.source, &plugin.name) else {
            continue;
        };
        let plugin_subdir = match normalize_skills_path(&source_path) {
            Ok(path) => path,
            Err(e) => {
                warn!(
                    plugin = %plugin.name,
                    path = %source_path,
                    error = %e,
                    "Rejected .agents marketplace plugin path"
                );
                continue;
            }
        };
        components.push((plugin.name, plugin_path.join(plugin_subdir)));
    }

    Some(components)
}

fn read_first_marketplace(plugin_path: &Path) -> Option<(PathBuf, String)> {
    for marketplace_path in PLUGIN_MARKETPLACE_PATHS {
        let path = plugin_path.join(marketplace_path);
        if let Ok(raw) = std::fs::read_to_string(&path) {
            return Some((path, raw));
        }
    }
    None
}

fn marketplace_local_path(source: &PluginMarketplaceSource, plugin_name: &str) -> Option<String> {
    match source {
        PluginMarketplaceSource::LocalPath(path) => {
            if path.trim().starts_with("./") {
                Some(path.clone())
            } else {
                debug!(
                    plugin = %plugin_name,
                    source = %path,
                    "Skipping non-local plugin marketplace string source"
                );
                None
            }
        }
        PluginMarketplaceSource::Descriptor { source, path } if source == "local" => path.clone(),
        PluginMarketplaceSource::Descriptor { source, .. } => {
            debug!(
                plugin = %plugin_name,
                source = %source,
                "Skipping non-local plugin marketplace descriptor source"
            );
            None
        }
    }
}

/// Every installed plugin's resolved `skills/` directory.
///
/// The staging above copies these into `/etc/codex/skills` for Codex only —
/// Claude reads the plugins in place through its own `PluginConfig`. So this is
/// the one place a plugin skill's `SKILL.md` is always found, whichever harness
/// is running, and it is what `agent::skills` reads bodies from.
///
/// Blocking: reads plugin manifests. Call it off the async worker.
pub(crate) fn plugin_skill_dirs(plugins: &[PluginDelivery]) -> Vec<PathBuf> {
    plugin_skill_dirs_under(Path::new(PLUGINS_DIR), plugins)
}

fn plugin_skill_dirs_under(plugins_root: &Path, plugins: &[PluginDelivery]) -> Vec<PathBuf> {
    if !plugins_root.is_dir() {
        return Vec::new();
    }
    let mut dirs = Vec::new();
    for plugin in plugins {
        let path = plugins_root.join(&plugin.name);
        if !path.is_dir() {
            continue;
        }
        for (_, root) in plugin_components(&path, &plugin.name) {
            let skills_dir = root.join(resolve_plugin_skills_subdir(&root));
            if skills_dir.is_dir() {
                dirs.push(skills_dir);
            }
        }
    }
    dirs
}

fn stage_plugin_component(
    plugin_path: &Path,
    plugin_name: &str,
    codex_skills_root: &Path,
) -> usize {
    let skills_subdir = resolve_plugin_skills_subdir(plugin_path);
    let skills_dir = plugin_path.join(&skills_subdir);
    if !skills_dir.is_dir() {
        debug!(
            plugin = %plugin_name,
            skills_dir = %skills_dir.display(),
            "Plugin has no skills directory — skipping"
        );
        return 0;
    }

    let entries = match std::fs::read_dir(&skills_dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                plugin = %plugin_name,
                error = %e,
                "Failed to read plugin skills directory"
            );
            return 0;
        }
    };

    let mut count = 0usize;
    for entry in entries.flatten() {
        let skill_src = entry.path();
        if !skill_src.is_dir() {
            continue;
        }
        let skill_name = match skill_src.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        let target_name = format!("{plugin_name}--{skill_name}");
        let target = codex_skills_root.join(&target_name);

        if target.exists() {
            if let Err(e) = std::fs::remove_dir_all(&target) {
                warn!(
                    plugin = %plugin_name,
                    skill = %skill_name,
                    error = %e,
                    "Failed to remove stale Codex skill target"
                );
                continue;
            }
        }
        if let Err(e) = std::fs::create_dir_all(&target) {
            warn!(
                plugin = %plugin_name,
                skill = %skill_name,
                error = %e,
                "Failed to create Codex skill target"
            );
            continue;
        }
        match copy_dir_contents(&skill_src, &target) {
            Ok(_) => {
                count += 1;
            }
            Err(e) => {
                warn!(
                    plugin = %plugin_name,
                    skill = %skill_name,
                    error = %e,
                    "Failed to copy plugin skill into Codex skills dir"
                );
            }
        }
    }
    count
}

fn resolve_plugin_skills_subdir(plugin_path: &Path) -> PathBuf {
    for manifest_path in PLUGIN_MANIFEST_PATHS {
        let manifest_path = plugin_path.join(manifest_path);
        let raw = match std::fs::read_to_string(&manifest_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match serde_json::from_str(&raw) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    manifest = %manifest_path.display(),
                    error = %e,
                    "Invalid plugin manifest — falling back to default skills/ dir"
                );
                return PathBuf::from("skills");
            }
        };
        let raw_path = manifest.skills.as_deref().unwrap_or("skills");
        match normalize_skills_path(raw_path) {
            Ok(path) => return PathBuf::from(path),
            Err(e) => {
                warn!(
                    manifest = %manifest_path.display(),
                    error = %e,
                    "Rejected plugin.json skills path — falling back to default skills/ dir"
                );
                return PathBuf::from("skills");
            }
        }
    }

    PathBuf::from("skills")
}

/// Normalize a `plugin.json` `skills` value:
///
/// - strips an optional leading `./`
/// - strips an optional trailing `/`
/// - rejects absolute paths (`/…`) and any `..` segment
///
/// Ported verbatim from `backend/src/skill/fetcher.rs::normalize_skills_path`.
fn normalize_skills_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('/') {
        return Err("plugin.json skills path must be repo-relative".to_string());
    }
    let stripped = trimmed
        .strip_prefix("./")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if stripped.split('/').any(|segment| segment == "..") {
        return Err("plugin.json skills path must not contain '..' segments".to_string());
    }
    Ok(stripped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn normalize_skills_path_defaults_and_strips() {
        assert_eq!(normalize_skills_path("skills").unwrap(), "skills");
        assert_eq!(normalize_skills_path("./skills").unwrap(), "skills");
        assert_eq!(
            normalize_skills_path("custom/path/").unwrap(),
            "custom/path"
        );
        assert_eq!(
            normalize_skills_path("  nested/dir  ").unwrap(),
            "nested/dir"
        );
    }

    #[test]
    fn normalize_skills_path_rejects_absolute_and_parent() {
        assert!(normalize_skills_path("/abs/path").is_err());
        assert!(normalize_skills_path("../escape").is_err());
        assert!(normalize_skills_path("nested/../escape").is_err());
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn delivered(name: &str) -> PluginDelivery {
        PluginDelivery {
            name: name.to_string(),
            git_url: format!("https://example.test/{name}.git"),
            skill_names: vec!["picked".to_string()],
            frontload_skill_names: Vec::new(),
        }
    }

    /// The dirs `agent::skills` reads plugin skill bodies from — the default
    /// layout, a `plugin.json` custom path, and nothing for a plugin with no
    /// skills dir at all.
    #[test]
    fn plugin_skill_dirs_resolves_default_and_custom_layouts() {
        let root = tempdir().unwrap();
        let plugins = root.path().join("plugins");
        write(&plugins.join("plugin-a/skills/foo/SKILL.md"), "foo body");
        write(
            &plugins.join("plugin-b/.claude-plugin/plugin.json"),
            r#"{"name":"plugin-b","skills":"./custom/skills/"}"#,
        );
        write(&plugins.join("plugin-b/custom/skills/baz/SKILL.md"), "baz");
        fs::create_dir_all(plugins.join("plugin-c")).unwrap();

        let dirs =
            plugin_skill_dirs_under(&plugins, &[delivered("plugin-b"), delivered("plugin-a")]);
        assert_eq!(
            dirs,
            vec![
                plugins.join("plugin-b/custom/skills"),
                plugins.join("plugin-a/skills"),
            ]
        );
    }

    #[test]
    fn plugin_skill_dirs_is_empty_without_a_plugins_root() {
        let root = tempdir().unwrap();
        assert!(
            plugin_skill_dirs_under(&root.path().join("nope"), &[delivered("plugin-a")]).is_empty()
        );
    }

    #[test]
    fn stages_default_and_custom_skills_paths_with_namespacing() {
        let root = tempdir().unwrap();
        let plugins = root.path().join("plugins");
        let codex = root.path().join("codex-skills");

        // Plugin A: default `skills/` layout, two skills.
        let plugin_a = plugins.join("plugin-a");
        write(
            &plugin_a.join("skills/foo/SKILL.md"),
            "---\nname: foo\n---\nfoo body",
        );
        write(&plugin_a.join("skills/foo/reference.md"), "ref");
        write(
            &plugin_a.join("skills/bar/SKILL.md"),
            "---\nname: bar\n---\nbar body",
        );

        // Plugin B: custom skills path via plugin.json, one skill.
        let plugin_b = plugins.join("plugin-b");
        write(
            &plugin_b.join(".claude-plugin/plugin.json"),
            r#"{"name":"plugin-b","skills":"./custom/skills/"}"#,
        );
        write(
            &plugin_b.join("custom/skills/baz/SKILL.md"),
            "---\nname: baz\n---\nbaz body",
        );

        // Plugin C: no skills dir at all — must be ignored.
        fs::create_dir_all(plugins.join("plugin-c")).unwrap();

        // Plugin D: Codex manifest location.
        let plugin_d = plugins.join("plugin-d");
        write(
            &plugin_d.join(".codex-plugin/plugin.json"),
            r#"{"name":"plugin-d","skills":"./codex-skills/"}"#,
        );
        write(
            &plugin_d.join("codex-skills/qux/SKILL.md"),
            "---\nname: qux\n---\nqux body",
        );

        let count = stage_plugin_skills_with_roots(&plugins, &codex);
        assert_eq!(count, 4);

        assert_eq!(
            fs::read_to_string(codex.join("plugin-a--foo/SKILL.md")).unwrap(),
            "---\nname: foo\n---\nfoo body"
        );
        assert_eq!(
            fs::read_to_string(codex.join("plugin-a--foo/reference.md")).unwrap(),
            "ref"
        );
        assert!(codex.join("plugin-a--bar/SKILL.md").is_file());
        assert!(codex.join("plugin-b--baz/SKILL.md").is_file());
        assert!(codex.join("plugin-d--qux/SKILL.md").is_file());
        assert!(!codex.join("plugin-c").exists());
    }

    #[test]
    fn stages_agents_marketplace_local_plugins() {
        let root = tempdir().unwrap();
        let plugins = root.path().join("plugins");
        let codex = root.path().join("codex-skills");

        let marketplace_repo = plugins.join("marketplace");
        write(
            &marketplace_repo.join(".agents/plugins/marketplace.json"),
            r#"{
                "name": "local",
                "plugins": [
                    {
                        "name": "moatless-skills",
                        "source": {
                            "source": "local",
                            "path": "./plugins/moatless-skills"
                        },
                        "policy": {
                            "installation": "AVAILABLE",
                            "authentication": "ON_INSTALL"
                        },
                        "category": "Development"
                    },
                    {
                        "name": "remote-only",
                        "source": {
                            "source": "url",
                            "url": "git@github.com:example/remote-only.git"
                        },
                        "policy": {
                            "installation": "AVAILABLE",
                            "authentication": "ON_INSTALL"
                        },
                        "category": "Development"
                    }
                ]
            }"#,
        );
        write(
            &marketplace_repo.join("plugins/moatless-skills/.codex-plugin/plugin.json"),
            r#"{"name":"moatless-skills","skills":"./skills/"}"#,
        );
        write(
            &marketplace_repo.join("plugins/moatless-skills/skills/sample-skill/SKILL.md"),
            "---\nname: sample-skill\n---\nbody",
        );

        let count = stage_plugin_skills_with_roots(&plugins, &codex);
        assert_eq!(count, 1);
        assert!(
            codex
                .join("moatless-skills--sample-skill/SKILL.md")
                .is_file()
        );
        assert!(!codex.join("remote-only--sample-skill/SKILL.md").exists());
    }

    #[test]
    fn stages_claude_marketplace_string_source_plugins() {
        let root = tempdir().unwrap();
        let plugins = root.path().join("plugins");
        let codex = root.path().join("codex-skills");

        let marketplace_repo = plugins.join("claude-marketplace");
        write(
            &marketplace_repo.join(".claude-plugin/marketplace.json"),
            r#"{
                "$schema": "https://anthropic.com/claude-code/marketplace.schema.json",
                "name": "local",
                "plugins": [
                    {
                        "name": "moatless-skills",
                        "description": "General development workflows",
                        "author": {"name": "Albert Örwall"},
                        "source": "./plugins/moatless-skills",
                        "category": "development"
                    }
                ]
            }"#,
        );
        write(
            &marketplace_repo.join("plugins/moatless-skills/.claude-plugin/plugin.json"),
            r#"{"name":"moatless-skills","skills":"./skills/"}"#,
        );
        write(
            &marketplace_repo.join("plugins/moatless-skills/skills/code-review/SKILL.md"),
            "---\nname: code-review\n---\nbody",
        );

        let count = stage_plugin_skills_with_roots(&plugins, &codex);
        assert_eq!(count, 1);
        assert!(
            codex
                .join("moatless-skills--code-review/SKILL.md")
                .is_file()
        );
    }

    #[test]
    fn rerun_replaces_stale_skill_content() {
        let root = tempdir().unwrap();
        let plugins = root.path().join("plugins");
        let codex = root.path().join("codex-skills");

        let plugin = plugins.join("plug");
        write(&plugin.join("skills/alpha/SKILL.md"), "v1");
        assert_eq!(stage_plugin_skills_with_roots(&plugins, &codex), 1);
        assert_eq!(
            fs::read_to_string(codex.join("plug--alpha/SKILL.md")).unwrap(),
            "v1"
        );

        fs::write(plugin.join("skills/alpha/SKILL.md"), "v2").unwrap();

        assert_eq!(stage_plugin_skills_with_roots(&plugins, &codex), 1);
        assert_eq!(
            fs::read_to_string(codex.join("plug--alpha/SKILL.md")).unwrap(),
            "v2"
        );
    }

    #[test]
    fn missing_plugins_root_is_noop() {
        let root = tempdir().unwrap();
        let plugins = root.path().join("does-not-exist");
        let codex = root.path().join("codex-skills");
        assert_eq!(stage_plugin_skills_with_roots(&plugins, &codex), 0);
        assert!(!codex.exists());
    }

    #[test]
    fn invalid_manifest_falls_back_to_default() {
        let root = tempdir().unwrap();
        let plugins = root.path().join("plugins");
        let codex = root.path().join("codex-skills");
        let plugin = plugins.join("plug");
        write(&plugin.join(".claude-plugin/plugin.json"), "{ not json");
        write(&plugin.join("skills/fallback/SKILL.md"), "ok");

        assert_eq!(stage_plugin_skills_with_roots(&plugins, &codex), 1);
        assert!(codex.join("plug--fallback/SKILL.md").is_file());
    }

    #[test]
    fn rejects_absolute_manifest_path_and_falls_back() {
        let root = tempdir().unwrap();
        let plugins = root.path().join("plugins");
        let codex = root.path().join("codex-skills");
        let plugin = plugins.join("plug");
        write(
            &plugin.join(".claude-plugin/plugin.json"),
            r#"{"skills":"/etc/passwd"}"#,
        );
        write(&plugin.join("skills/ok/SKILL.md"), "ok");

        assert_eq!(stage_plugin_skills_with_roots(&plugins, &codex), 1);
        assert!(codex.join("plug--ok/SKILL.md").is_file());
    }

    // ---- Pruning to the picked set ----

    #[test]
    fn keep_only_removes_the_skills_nobody_picked() {
        let root = tempdir().unwrap();
        let plugin = root.path().join("plugin-a");
        write(&plugin.join("skills/alpha/SKILL.md"), "alpha");
        write(&plugin.join("skills/beta/SKILL.md"), "beta");
        write(&plugin.join("skills/beta/reference.md"), "ref");

        let dropped = keep_only(&plugin, "plugin-a", &["alpha".to_string()]);

        assert_eq!(dropped, 1);
        assert!(plugin.join("skills/alpha/SKILL.md").exists());
        assert!(
            !plugin.join("skills/beta").exists(),
            "every file of an unpicked skill goes, not just its SKILL.md"
        );
    }

    #[test]
    fn keep_only_leaves_everything_that_is_not_a_skill() {
        let root = tempdir().unwrap();
        let plugin = root.path().join("plugin-a");
        write(
            &plugin.join(".claude-plugin/plugin.json"),
            r#"{"name":"a"}"#,
        );
        write(&plugin.join("commands/deploy.md"), "deploy");
        write(&plugin.join("hooks/pre-commit.sh"), "hook");
        write(&plugin.join("agents/reviewer.md"), "reviewer");
        write(&plugin.join("README.md"), "readme");
        write(&plugin.join("skills/README.md"), "skills readme");
        write(&plugin.join("skills/alpha/SKILL.md"), "alpha");

        keep_only(&plugin, "plugin-a", &[]);

        assert!(!plugin.join("skills/alpha").exists());
        for kept in [
            ".claude-plugin/plugin.json",
            "commands/deploy.md",
            "hooks/pre-commit.sh",
            "agents/reviewer.md",
            "README.md",
            // A file sitting in the skills root belongs to no skill, so nobody
            // can have unpicked it.
            "skills/README.md",
        ] {
            assert!(
                plugin.join(kept).exists(),
                "a plugin still has to work — only its unpicked skills are removed: {kept}"
            );
        }
    }

    #[test]
    fn keep_only_honours_a_custom_skills_path() {
        let root = tempdir().unwrap();
        let plugin = root.path().join("plugin-b");
        write(
            &plugin.join(".claude-plugin/plugin.json"),
            r#"{"name":"plugin-b","skills":"./custom/skills/"}"#,
        );
        write(&plugin.join("custom/skills/alpha/SKILL.md"), "alpha");
        write(&plugin.join("custom/skills/beta/SKILL.md"), "beta");

        let dropped = keep_only(&plugin, "plugin-b", &["beta".to_string()]);

        assert_eq!(dropped, 1);
        assert!(!plugin.join("custom/skills/alpha").exists());
        assert!(plugin.join("custom/skills/beta/SKILL.md").exists());
    }

    #[test]
    fn keep_only_prunes_every_component_of_a_marketplace_repo() {
        let root = tempdir().unwrap();
        let plugin = root.path().join("marketplace-repo");
        write(
            &plugin.join(".agents/plugins/marketplace.json"),
            r#"{"plugins":[
                {"name":"one","source":{"source":"local","path":"./plugins/one"}},
                {"name":"two","source":{"source":"local","path":"./plugins/two"}}
            ]}"#,
        );
        write(&plugin.join("plugins/one/skills/alpha/SKILL.md"), "alpha");
        write(&plugin.join("plugins/one/skills/beta/SKILL.md"), "beta");
        write(&plugin.join("plugins/two/skills/gamma/SKILL.md"), "gamma");

        let dropped = keep_only(&plugin, "marketplace-repo", &["alpha".to_string()]);

        assert_eq!(
            dropped, 2,
            "an unpicked skill goes wherever in the repo it lives"
        );
        assert!(plugin.join("plugins/one/skills/alpha/SKILL.md").exists());
        assert!(!plugin.join("plugins/one/skills/beta").exists());
        assert!(!plugin.join("plugins/two/skills/gamma").exists());
    }

    #[test]
    fn keep_only_on_a_plugin_without_skills_does_nothing() {
        let root = tempdir().unwrap();
        let plugin = root.path().join("plugin-c");
        write(&plugin.join("commands/deploy.md"), "deploy");

        assert_eq!(keep_only(&plugin, "plugin-c", &["alpha".to_string()]), 0);
        assert!(plugin.join("commands/deploy.md").exists());
    }

    #[test]
    fn keep_only_is_idempotent() {
        let root = tempdir().unwrap();
        let plugin = root.path().join("plugin-a");
        write(&plugin.join("skills/alpha/SKILL.md"), "alpha");
        write(&plugin.join("skills/beta/SKILL.md"), "beta");

        let picked = ["alpha".to_string()];
        assert_eq!(keep_only(&plugin, "plugin-a", &picked), 1);
        assert_eq!(
            keep_only(&plugin, "plugin-a", &picked),
            0,
            "a re-started sandbox re-runs the prune over an already-cloned plugin"
        );
        assert!(plugin.join("skills/alpha/SKILL.md").exists());
    }
}
