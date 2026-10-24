//! The `skills` key of a `.moatless/workspaces.json` entry: which Plugin
//! Skills a git-declared Workspace turns on or off.
//!
//! A present key makes git authoritative for that Workspace's own
//! `plugin_activations` rows; an absent one leaves them alone. Only the
//! Workspace reach is declared — everyone's and a person's need an authority
//! commit access does not carry.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::plugin::models::{DeclaredActivation, Plugin, PluginSkill};
use crate::workspace::git_sync::UnknownKeys;
use moatless_workspace_types::schemas::workspace::WorkspaceSyncWarning;

/// `"skills": { "enable": [...], "disable": [...] }`, each name a registered
/// Plugin's `name` or `<plugin>:<skill>`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillActivations {
    #[serde(default)]
    pub enable: Vec<String>,
    #[serde(default)]
    pub disable: Vec<String>,
    #[serde(flatten)]
    pub unknown: UnknownKeys,
}

const MALFORMED_NAME: &str = "not a <plugin> or <plugin>:<skill> name, ignored";
const UNKNOWN_PLUGIN: &str = "no registered plugin has this name, ignored";
const UNKNOWN_SKILL: &str =
    "the plugin has no stored skill by this name; recorded, inert until it has one";
const CONFLICTING: &str = "named in both enable and disable, ignored";

/// The records `declared` asks for, and a warning for each name it cannot
/// honour. `path` is the JSON path of the `skills` object.
///
/// A skill the plugin's stored list lacks is still recorded rather than
/// dropped: that list is refreshed only when the plugin syncs, so a skill added
/// upstream since would otherwise wait for a second workspace sync.
pub(crate) fn resolve_declared(
    declared: &SkillActivations,
    plugins: &[Plugin],
    skills: &[PluginSkill],
    path: &str,
) -> (Vec<DeclaredActivation>, Vec<WorkspaceSyncWarning>) {
    let mut warnings = Vec::new();
    let mut warn = |path: String, message: &str| {
        warnings.push(WorkspaceSyncWarning {
            path,
            message: message.to_string(),
        });
    };

    // Keyed on the target so a name listed twice writes one record, and one
    // listed on both sides is caught before either is written.
    let mut wanted: BTreeMap<(String, Option<String>), (bool, Option<String>)> = BTreeMap::new();
    let mut conflicting = Vec::new();
    let lists = [
        ("enable", &declared.enable, true),
        ("disable", &declared.disable, false),
    ];
    for (list, names, enabled) in lists {
        for (j, name) in names.iter().enumerate() {
            let at = format!("{path}.{list}[{j}]");
            let Some((plugin_name, skill_name)) = parse_name(name) else {
                warn(at, MALFORMED_NAME);
                continue;
            };
            let Some(plugin) = plugins.iter().find(|p| p.name == plugin_name) else {
                warn(at, UNKNOWN_PLUGIN);
                continue;
            };
            let key = (plugin.id.clone(), skill_name.map(str::to_string));
            match wanted.get(&key) {
                Some((seen, _)) if *seen != enabled => conflicting.push((key, at)),
                Some(_) => {}
                None => {
                    let unknown_skill = skill_name.filter(|skill| {
                        !skills
                            .iter()
                            .any(|s| s.plugin_id == plugin.id && s.name == *skill)
                    });
                    wanted.insert(key, (enabled, unknown_skill.map(|_| at)));
                }
            }
        }
    }

    for (key, at) in conflicting {
        if wanted.remove(&key).is_some() {
            warn(at, CONFLICTING);
        }
    }
    let mut activations = Vec::with_capacity(wanted.len());
    for ((plugin_id, skill_name), (enabled, unknown_skill_at)) in wanted {
        if let Some(at) = unknown_skill_at {
            warn(at, UNKNOWN_SKILL);
        }
        activations.push(DeclaredActivation {
            plugin_id,
            skill_name,
            enabled,
        });
    }
    (activations, warnings)
}

/// `plugin` or `plugin:skill`, both halves non-empty.
fn parse_name(name: &str) -> Option<(&str, Option<&str>)> {
    let name = name.trim();
    match name.split_once(':') {
        Some((plugin, skill))
            if !plugin.is_empty() && !skill.is_empty() && !skill.contains(':') =>
        {
            Some((plugin, Some(skill)))
        }
        Some(_) => None,
        None if !name.is_empty() => Some((name, None)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(id: &str, name: &str) -> Plugin {
        Plugin {
            id: id.to_string(),
            name: name.to_string(),
            git_url: format!("https://github.com/acme/{name}"),
            source: moatless_workspace_types::plugin::PluginSource::Manual,
            synced_version: None,
            synced_at: None,
            disabled_at: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn skill(plugin_id: &str, name: &str) -> PluginSkill {
        PluginSkill {
            plugin_id: plugin_id.to_string(),
            name: name.to_string(),
            description: String::new(),
            path: format!("skills/{name}/SKILL.md"),
        }
    }

    fn declared(json: &str) -> SkillActivations {
        serde_json::from_str(json).expect("parse skills")
    }

    fn resolve(json: &str) -> (Vec<DeclaredActivation>, Vec<(String, String)>) {
        let plugins = [plugin("p1", "general"), plugin("p2", "backend")];
        let skills = [skill("p1", "writing-style"), skill("p2", "java-testing")];
        let (activations, warnings) =
            resolve_declared(&declared(json), &plugins, &skills, "workspaces[0].skills");
        let warnings = warnings.into_iter().map(|w| (w.path, w.message)).collect();
        (activations, warnings)
    }

    fn record(plugin_id: &str, skill_name: Option<&str>, enabled: bool) -> DeclaredActivation {
        DeclaredActivation {
            plugin_id: plugin_id.to_string(),
            skill_name: skill_name.map(str::to_string),
            enabled,
        }
    }

    #[test]
    fn plugin_and_skill_names_resolve_to_workspace_records() {
        let (activations, warnings) = resolve(
            r#"{ "enable": ["general", "backend:java-testing"], "disable": ["general:writing-style"] }"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            activations,
            [
                record("p1", None, true),
                record("p1", Some("writing-style"), false),
                record("p2", Some("java-testing"), true),
            ]
        );
    }

    /// One bad name must not cost the Workspace the rest of its list.
    #[test]
    fn an_unknown_plugin_or_malformed_name_is_named_and_skipped() {
        let (activations, warnings) =
            resolve(r#"{ "enable": ["frontend", "general", ":x", "a:b:c", " "] }"#);

        assert_eq!(activations, [record("p1", None, true)]);
        assert_eq!(
            warnings,
            [
                (
                    "workspaces[0].skills.enable[0]".to_string(),
                    UNKNOWN_PLUGIN.to_string()
                ),
                (
                    "workspaces[0].skills.enable[2]".to_string(),
                    MALFORMED_NAME.to_string()
                ),
                (
                    "workspaces[0].skills.enable[3]".to_string(),
                    MALFORMED_NAME.to_string()
                ),
                (
                    "workspaces[0].skills.enable[4]".to_string(),
                    MALFORMED_NAME.to_string()
                ),
            ]
        );
    }

    /// The stored skill list lags the plugin's source, so an unknown skill is
    /// recorded and only reported.
    #[test]
    fn a_skill_the_stored_list_lacks_is_recorded_and_named() {
        let (activations, warnings) = resolve(r#"{ "enable": ["general:brand-new"] }"#);

        assert_eq!(activations, [record("p1", Some("brand-new"), true)]);
        assert_eq!(
            warnings,
            [(
                "workspaces[0].skills.enable[0]".to_string(),
                UNKNOWN_SKILL.to_string()
            )]
        );
    }

    #[test]
    fn a_target_on_both_sides_writes_neither_and_a_repeat_writes_one() {
        let (activations, warnings) =
            resolve(r#"{ "enable": ["general", "backend", "backend"], "disable": ["general"] }"#);

        assert_eq!(activations, [record("p2", None, true)]);
        assert_eq!(
            warnings,
            [(
                "workspaces[0].skills.disable[0]".to_string(),
                CONFLICTING.to_string()
            )]
        );
    }

    /// `{}` is a present key: git owns the Workspace's records and declares
    /// none, which clears any the API wrote.
    #[test]
    fn an_empty_object_declares_no_records() {
        let (activations, warnings) = resolve("{}");

        assert!(activations.is_empty());
        assert!(warnings.is_empty());
    }
}
