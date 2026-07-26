//! What a person actually gets: activation records in, picked skills out.
//!
//! Both functions here are pure. Resolution takes the records and the stored
//! skills and returns the effective set; assembly takes that set and a plugin's
//! file listing and returns the listing to deliver. Neither touches the
//! database, the network or the filesystem, which is what makes the rules that
//! matter cheap to pin down in tests.

use std::collections::{BTreeMap, BTreeSet};

/// Who an activation record applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    /// Everyone — the default position, set by an administrator.
    Everyone,
    /// One person, overriding whatever the default says.
    Personal,
}

/// One stored activation record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub plugin_id: String,
    /// `None` means the whole plugin.
    pub skill_name: Option<String>,
    pub reach: Reach,
    pub enabled: bool,
}

/// A plugin and the skills stored for it, as resolution sees it. A plugin that
/// has been removed simply is not in the list, which is why a record pointing
/// at one cannot resurrect it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSkills {
    pub plugin_id: String,
    pub skill_names: Vec<String>,
}

/// One plugin a person gets, and which of its skills come with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlugin {
    pub plugin_id: String,
    /// Always non-empty: a plugin with no picked skills is not delivered.
    pub skill_names: Vec<String>,
}

/// Precedence, most specific first. A personal record beats a global one at
/// every level, so somebody can turn off what was turned on for everyone; and
/// within one reach a skill record beats the plugin record, so "the whole
/// plugin except this one skill" is expressible.
fn precedence(reach: Reach, skill_specific: bool) -> u8 {
    match (reach, skill_specific) {
        (Reach::Personal, true) => 4,
        (Reach::Personal, false) => 3,
        (Reach::Everyone, true) => 2,
        (Reach::Everyone, false) => 1,
    }
}

/// Resolve what one person gets.
///
/// `activations` must already be narrowed to the global records plus that one
/// person's own; this function does not know who anybody is.
pub fn resolve(plugins: &[PluginSkills], activations: &[Activation]) -> Vec<ResolvedPlugin> {
    let mut resolved = Vec::new();

    for plugin in plugins {
        // For each skill: the winning record's `enabled`, and the precedence it
        // won at, so a later record only displaces it if it is more specific.
        let mut decided: BTreeMap<&str, (u8, bool)> = BTreeMap::new();

        for act in activations
            .iter()
            .filter(|a| a.plugin_id == plugin.plugin_id)
        {
            let rank = precedence(act.reach, act.skill_name.is_some());

            let targets: Vec<&str> = match act.skill_name.as_deref() {
                // A plugin-level record speaks for every skill the plugin has
                // now — including ones added by a sync after the record was
                // written, which is what "turn this plugin on" ought to mean.
                None => plugin.skill_names.iter().map(String::as_str).collect(),
                // A record naming a skill the plugin no longer has is inert
                // rather than an error: the skill went away upstream.
                Some(name) => plugin
                    .skill_names
                    .iter()
                    .filter(|s| s.as_str() == name)
                    .map(String::as_str)
                    .collect(),
            };

            for skill in targets {
                let beats = decided.get(skill).is_none_or(|(seen, _)| rank > *seen);
                if beats {
                    decided.insert(skill, (rank, act.enabled));
                }
            }
        }

        let skill_names: Vec<String> = plugin
            .skill_names
            .iter()
            .filter(|s| decided.get(s.as_str()).is_some_and(|(_, on)| *on))
            .cloned()
            .collect();

        if !skill_names.is_empty() {
            resolved.push(ResolvedPlugin {
                plugin_id: plugin.plugin_id.clone(),
                skill_names,
            });
        }
    }

    resolved
}

/// Decide which of a plugin's files to deliver.
///
/// `paths` is the plugin's listing, repo-relative. `skills_prefix` is where its
/// skills live (`skills` for the default layout). Everything that is not a
/// skill — the manifest, commands, hooks, agents — is kept whatever happens;
/// only skill directories nobody picked are dropped.
pub fn assemble<'a>(
    paths: &[&'a str],
    skills_prefix: &str,
    picked: &BTreeSet<String>,
) -> Vec<&'a str> {
    let prefix = format!("{}/", skills_prefix.trim_end_matches('/'));

    paths
        .iter()
        .copied()
        .filter(|path| match skill_of(path, &prefix) {
            Some(skill) => picked.contains(skill),
            // Not under the skills root at all — not ours to drop.
            None => true,
        })
        .collect()
}

/// The skill directory a path belongs to, if it is under the skills root.
///
/// A file sitting directly in the skills root (`skills/README.md`) belongs to
/// no skill and is kept.
fn skill_of<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(prefix)?;
    let (dir, tail) = rest.split_once('/')?;
    if tail.is_empty() || dir.is_empty() {
        return None;
    }
    Some(dir)
}

/// Group stored skills and records into the shape [`resolve`] wants, then
/// resolve. Kept next to the rules it feeds so the grouping cannot drift from
/// them.
pub fn resolve_rows(
    plugins: &[crate::plugin::models::Plugin],
    skills: &[crate::plugin::models::PluginSkill],
    rows: &[crate::plugin::models::ActivationRow],
) -> Vec<ResolvedPlugin> {
    let plugin_skills: Vec<PluginSkills> = plugins
        .iter()
        .map(|p| PluginSkills {
            plugin_id: p.id.clone(),
            skill_names: skills
                .iter()
                .filter(|s| s.plugin_id == p.id)
                .map(|s| s.name.clone())
                .collect(),
        })
        .collect();

    let activations: Vec<Activation> = rows
        .iter()
        .map(|r| Activation {
            plugin_id: r.plugin_id.clone(),
            skill_name: r.skill_name.clone(),
            reach: if r.user_id.is_some() {
                Reach::Personal
            } else {
                Reach::Everyone
            },
            enabled: r.enabled,
        })
        .collect();

    resolve(&plugin_skills, &activations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(id: &str, skills: &[&str]) -> PluginSkills {
        PluginSkills {
            plugin_id: id.to_string(),
            skill_names: skills.iter().copied().map(String::from).collect(),
        }
    }

    fn act(plugin_id: &str, skill: Option<&str>, reach: Reach, enabled: bool) -> Activation {
        Activation {
            plugin_id: plugin_id.to_string(),
            skill_name: skill.map(str::to_string),
            reach,
            enabled,
        }
    }

    fn names(resolved: &[ResolvedPlugin], plugin_id: &str) -> Vec<String> {
        resolved
            .iter()
            .find(|r| r.plugin_id == plugin_id)
            .map(|r| r.skill_names.clone())
            .unwrap_or_default()
    }

    // ---- Resolution ----

    #[test]
    fn nothing_is_delivered_without_a_record() {
        let resolved = resolve(&[plugin("p1", &["a", "b"])], &[]);
        assert!(
            resolved.is_empty(),
            "a registered plugin nobody turned on reaches nobody"
        );
    }

    #[test]
    fn a_global_plugin_record_delivers_every_skill_it_has() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b"])],
            &[act("p1", None, Reach::Everyone, true)],
        );
        assert_eq!(names(&resolved, "p1"), vec!["a", "b"]);
    }

    #[test]
    fn a_global_skill_record_delivers_only_that_skill() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b"])],
            &[act("p1", Some("a"), Reach::Everyone, true)],
        );
        assert_eq!(
            names(&resolved, "p1"),
            vec!["a"],
            "picking one skill must not drag its siblings along"
        );
    }

    #[test]
    fn a_personal_record_switches_a_global_one_off() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b"])],
            &[
                act("p1", None, Reach::Everyone, true),
                act("p1", None, Reach::Personal, false),
            ],
        );
        assert!(
            resolved.is_empty(),
            "a plugin that gets in somebody's way has to be escapable"
        );
    }

    #[test]
    fn a_personal_record_switches_a_global_one_on() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                act("p1", None, Reach::Everyone, false),
                act("p1", None, Reach::Personal, true),
            ],
        );
        assert_eq!(
            names(&resolved, "p1"),
            vec!["a"],
            "personal wins in both directions, not just off"
        );
    }

    #[test]
    fn a_global_record_arriving_after_a_personal_one_changes_nothing() {
        let personal_only = resolve(
            &[plugin("p1", &["a"])],
            &[act("p1", None, Reach::Personal, true)],
        );
        let then_global = resolve(
            &[plugin("p1", &["a"])],
            &[
                act("p1", None, Reach::Personal, true),
                act("p1", None, Reach::Everyone, true),
            ],
        );
        assert_eq!(
            personal_only, then_global,
            "an administrator turning on what somebody already had must be a \
             no-op for that person"
        );
    }

    #[test]
    fn a_personal_skill_record_carves_one_skill_out_of_a_global_plugin() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b", "c"])],
            &[
                act("p1", None, Reach::Everyone, true),
                act("p1", Some("b"), Reach::Personal, false),
            ],
        );
        assert_eq!(names(&resolved, "p1"), vec!["a", "c"]);
    }

    #[test]
    fn a_personal_plugin_record_beats_a_global_skill_record() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b"])],
            &[
                act("p1", Some("a"), Reach::Everyone, true),
                act("p1", None, Reach::Personal, false),
            ],
        );
        assert!(
            resolved.is_empty(),
            "turning the whole plugin off for myself has to override a global \
             record however specific it is"
        );
    }

    #[test]
    fn a_removed_plugin_disappears_regardless_of_records_pointing_at_it() {
        let resolved = resolve(
            // p1 is gone: it is not in the plugin list any more.
            &[plugin("p2", &["x"])],
            &[
                act("p1", None, Reach::Everyone, true),
                act("p1", Some("a"), Reach::Personal, true),
            ],
        );
        assert!(
            resolved.is_empty(),
            "records outliving their plugin must not deliver anything"
        );
    }

    #[test]
    fn a_record_naming_a_skill_the_plugin_no_longer_has_is_inert() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                act("p1", Some("a"), Reach::Everyone, true),
                act("p1", Some("removed-upstream"), Reach::Personal, true),
            ],
        );
        assert_eq!(names(&resolved, "p1"), vec!["a"]);
    }

    #[test]
    fn a_skill_added_by_a_later_sync_is_covered_by_the_plugin_record() {
        let resolved = resolve(
            // "c" appeared after the record was written.
            &[plugin("p1", &["a", "b", "c"])],
            &[act("p1", None, Reach::Everyone, true)],
        );
        assert_eq!(
            names(&resolved, "p1"),
            vec!["a", "b", "c"],
            "\"turn this plugin on\" means the plugin, not the skills it had \
             on the day somebody said so"
        );
    }

    // ---- Assembly ----

    fn picked(names: &[&str]) -> BTreeSet<String> {
        names.iter().copied().map(String::from).collect()
    }

    #[test]
    fn assembly_drops_the_skills_nobody_picked() {
        let paths = vec![
            "skills/alpha/SKILL.md",
            "skills/beta/SKILL.md",
            "skills/beta/reference.md",
        ];
        assert_eq!(
            assemble(&paths, "skills", &picked(&["alpha"])),
            vec!["skills/alpha/SKILL.md"],
            "every file of an unpicked skill goes, not just its SKILL.md"
        );
    }

    #[test]
    fn assembly_keeps_everything_that_is_not_a_skill() {
        let paths = vec![
            ".claude-plugin/plugin.json",
            "commands/deploy.md",
            "hooks/pre-commit.sh",
            "agents/reviewer.md",
            "README.md",
            "skills/alpha/SKILL.md",
            "skills/beta/SKILL.md",
        ];
        assert_eq!(
            assemble(&paths, "skills", &picked(&["alpha"])),
            vec![
                ".claude-plugin/plugin.json",
                "commands/deploy.md",
                "hooks/pre-commit.sh",
                "agents/reviewer.md",
                "README.md",
                "skills/alpha/SKILL.md",
            ],
            "a plugin still has to work — only its unpicked skills are removed"
        );
    }

    #[test]
    fn assembly_keeps_files_sitting_directly_in_the_skills_root() {
        let paths = vec!["skills/README.md", "skills/alpha/SKILL.md"];
        assert_eq!(
            assemble(&paths, "skills", &picked(&[])),
            vec!["skills/README.md"],
            "a file in the skills root belongs to no skill, so nobody can have \
             unpicked it"
        );
    }

    #[test]
    fn assembly_honours_a_non_default_skills_prefix() {
        let paths = vec![
            "plugins/moatless-skills/skills/alpha/SKILL.md",
            "plugins/moatless-skills/skills/beta/SKILL.md",
        ];
        assert_eq!(
            assemble(
                &paths,
                "plugins/moatless-skills/skills",
                &picked(&["beta"])
            ),
            vec!["plugins/moatless-skills/skills/beta/SKILL.md"]
        );
    }

    #[test]
    fn assembly_with_nothing_picked_still_leaves_a_working_plugin() {
        let paths = vec![".claude-plugin/plugin.json", "skills/alpha/SKILL.md"];
        assert_eq!(
            assemble(&paths, "skills", &picked(&[])),
            vec![".claude-plugin/plugin.json"]
        );
    }
}
