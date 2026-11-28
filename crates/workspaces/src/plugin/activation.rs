//! What a person actually gets: activation records in, picked skills out.
//!
//! Resolution is pure — it takes the records and the stored skills and returns
//! the effective set, touching neither the database, the network nor the
//! filesystem. That is what makes the rules that matter cheap to pin down in
//! tests. Applying the result to a plugin's files is the sandbox's job, in
//! `init::plugin_skills`.

use std::collections::BTreeMap;

/// Who an activation record applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    /// Everyone — the default position, set by an administrator.
    Everyone,
    /// One person, overriding whatever the default says.
    Personal,
    /// One Workspace, overriding both — the same `workspace > user > global`
    /// order credentials and run configuration resolve in.
    Workspace,
}

/// One stored activation record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub plugin_id: String,
    /// `None` means the whole plugin.
    pub skill_name: Option<String>,
    pub reach: Reach,
    pub enabled: bool,
    /// Lift this record above every record that is not `always`, whatever reach
    /// either of them carries. See [`precedence`].
    pub always: bool,
    /// Frontload the skill's body rather than leaving it to be picked for a
    /// turn. Read off whichever record decides the skill, so the record that
    /// says whether it arrives is the one that says how.
    pub frontload: bool,
}

/// A plugin and the skills stored for it, as resolution sees it. A plugin that
/// has been removed simply is not in the list, which is why a record pointing
/// at one cannot resurrect it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSkills {
    pub plugin_id: String,
    pub skill_names: Vec<String>,
}

/// The skills one plugin contributes, and how they arrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedSkills {
    /// Always non-empty: a plugin with no picked skills is not delivered.
    pub names: Vec<String>,
    /// The subset of [`names`](Self::names) whose deciding record asked for
    /// frontloading. A skill nobody delivers cannot be frontloaded, so this
    /// never names one that is not in `names`.
    pub frontloaded: Vec<String>,
}

/// One plugin a person gets, and which of its skills come with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlugin {
    pub plugin_id: String,
    pub skills: PickedSkills,
}

/// Precedence, most specific first: `workspace > personal > everyone`, skill
/// over plugin.
///
/// `always` sits outside that order rather than inside it: a record carrying it
/// outranks any record lacking it regardless of reach; scope order decides
/// among records that both carry it.
fn precedence(reach: Reach, skill_specific: bool, always: bool) -> u8 {
    let by_scope = match (reach, skill_specific) {
        (Reach::Workspace, true) => 6,
        (Reach::Workspace, false) => 5,
        (Reach::Personal, true) => 4,
        (Reach::Personal, false) => 3,
        (Reach::Everyone, true) => 2,
        (Reach::Everyone, false) => 1,
    };
    if always { by_scope + 6 } else { by_scope }
}

/// What the record that won a skill decided about it.
#[derive(Debug, Clone, Copy)]
struct Decision {
    enabled: bool,
    frontload: bool,
}

/// Resolve what one person gets in one Workspace.
///
/// `activations` must already be narrowed to the global, personal, and
/// Workspace records for that person — this function does not know who anybody
/// is.
pub fn resolve(plugins: &[PluginSkills], activations: &[Activation]) -> Vec<ResolvedPlugin> {
    let mut resolved = Vec::new();

    for plugin in plugins {
        // For each skill: what the winning record decided, and the precedence
        // it won at, so a later record only displaces it if it is more
        // specific.
        let mut decided: BTreeMap<&str, (u8, Decision)> = BTreeMap::new();

        for act in activations
            .iter()
            .filter(|a| a.plugin_id == plugin.plugin_id)
        {
            let rank = precedence(act.reach, act.skill_name.is_some(), act.always);

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
                    decided.insert(
                        skill,
                        (
                            rank,
                            Decision {
                                enabled: act.enabled,
                                frontload: act.frontload,
                            },
                        ),
                    );
                }
            }
        }

        let picked: Vec<(&String, Decision)> = plugin
            .skill_names
            .iter()
            .filter_map(|s| {
                decided
                    .get(s.as_str())
                    .filter(|(_, d)| d.enabled)
                    .map(|(_, d)| (s, *d))
            })
            .collect();

        if !picked.is_empty() {
            resolved.push(ResolvedPlugin {
                plugin_id: plugin.plugin_id.clone(),
                skills: PickedSkills {
                    frontloaded: picked
                        .iter()
                        .filter(|(_, d)| d.frontload)
                        .map(|(s, _)| (*s).clone())
                        .collect(),
                    names: picked.iter().map(|(s, _)| (*s).clone()).collect(),
                },
            });
        }
    }

    resolved
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
            reach: reach_of(r),
            enabled: r.enabled,
            always: r.always,
            frontload: r.frontload,
        })
        .collect();

    let mut resolved = resolve(&plugin_skills, &activations);
    with_prompt_skills(&plugin_skills, plugins, &mut resolved);
    resolved
}

/// The plugin holding the skills every task's system prompt tells the agent
/// to load (`crates/tasks/src/agent/prompt.rs`).
const PROMPT_SKILLS_PLUGIN: &str = "moatless-skills";
const PROMPT_SKILLS: [&str; 3] = ["artifacts", "artifact-design", "artifact-diagramming"];

/// Add [`PROMPT_SKILLS`] to [`PROMPT_SKILLS_PLUGIN`]'s picks whatever the
/// records say, deliberately: the prompt names them unconditionally, so a
/// workspace that picks other skills would otherwise get "Unknown skill".
fn with_prompt_skills(
    plugin_skills: &[PluginSkills],
    plugins: &[crate::plugin::models::Plugin],
    resolved: &mut Vec<ResolvedPlugin>,
) {
    let Some(plugin) = plugins
        .iter()
        .find(|p| p.name == PROMPT_SKILLS_PLUGIN)
        .and_then(|p| plugin_skills.iter().find(|s| s.plugin_id == p.id))
    else {
        return;
    };
    let index = if let Some(index) = resolved
        .iter()
        .position(|r| r.plugin_id == plugin.plugin_id)
    {
        index
    } else {
        resolved.push(ResolvedPlugin {
            plugin_id: plugin.plugin_id.clone(),
            skills: PickedSkills {
                names: Vec::new(),
                frontloaded: Vec::new(),
            },
        });
        resolved.len() - 1
    };
    let names = &mut resolved[index].skills.names;
    *names = plugin
        .skill_names
        .iter()
        .filter(|s| names.contains(s) || PROMPT_SKILLS.contains(&s.as_str()))
        .cloned()
        .collect();
    if names.is_empty() {
        resolved.remove(index);
    }
}

/// Which reach a stored row speaks with.
///
/// A row carries at most one subject (enforced by the
/// `plugin_activations_one_subject` constraint), so reach follows from which
/// column is filled in.
pub fn reach_of(row: &crate::plugin::models::ActivationRow) -> Reach {
    if row.workspace_id.is_some() {
        Reach::Workspace
    } else if row.user_id.is_some() {
        Reach::Personal
    } else {
        Reach::Everyone
    }
}

/// Read what one person gets in one Workspace, straight out of the database.
///
/// The single place this read happens — the effective-plugins endpoint,
/// repository skill list, and task provisioning all go through it, so display
/// and sandbox cannot drift. Does not sync, so the stored skill names can be
/// stale; [`crate::ports::WorkspaceProjection`] syncs before a delivery.
pub async fn resolve_for(
    plugins: &crate::plugin::Plugins,
    user_id: Option<uuid::Uuid>,
    workspace_id: Option<&str>,
) -> Result<Vec<(crate::plugin::models::Plugin, PickedSkills)>, sqlx::Error> {
    Ok(resolve_with_deselected_for(plugins, user_id, workspace_id)
        .await?
        .0)
}

/// [`resolve_for`], plus the plugins [`deselected`] names.
pub async fn resolve_with_deselected_for(
    plugins: &crate::plugin::Plugins,
    user_id: Option<uuid::Uuid>,
    workspace_id: Option<&str>,
) -> Result<
    (
        Vec<(crate::plugin::models::Plugin, PickedSkills)>,
        Vec<crate::plugin::models::Plugin>,
    ),
    sqlx::Error,
> {
    let plugins = plugins.store();
    let registered = plugins.list_all().await?;
    if registered.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let skills = plugins.list_all_skills().await?;
    let rows = plugins.list_activations_for(user_id, workspace_id).await?;

    // Resolved over the live plugins but deselected over all of them, so a
    // sandbox already holding a disabled plugin is told to remove it.
    let live: Vec<_> = registered
        .iter()
        .filter(|p| p.disabled_at.is_none())
        .cloned()
        .collect();
    let resolved = resolve_rows(&live, &skills, &rows);
    let removed = deselected(&registered, &skills, &resolved)
        .into_iter()
        .cloned()
        .collect();
    let picked = resolved
        .into_iter()
        .filter_map(|r| {
            registered
                .iter()
                .find(|p| p.id == r.plugin_id)
                .map(|p| (p.clone(), r.skills))
        })
        .collect();
    Ok((picked, removed))
}

/// The registered plugins with stored skills that `resolved` picks none of.
///
/// A plugin with no stored skills is never deselected: a sync that could not
/// read it leaves it there, and a sandbox told to remove it would delete a
/// copy whose skills nobody turned off.
pub fn deselected<'a>(
    registered: &'a [crate::plugin::models::Plugin],
    skills: &[crate::plugin::models::PluginSkill],
    resolved: &[ResolvedPlugin],
) -> Vec<&'a crate::plugin::models::Plugin> {
    registered
        .iter()
        .filter(|p| skills.iter().any(|s| s.plugin_id == p.id))
        .filter(|p| !resolved.iter().any(|r| r.plugin_id == p.id))
        .collect()
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
            always: false,
            frontload: false,
        }
    }

    /// The same record, marked to outrank everything that is not.
    fn always(act: Activation) -> Activation {
        Activation {
            always: true,
            ..act
        }
    }

    /// The same record, asking for the skill's body up front.
    fn frontloading(act: Activation) -> Activation {
        Activation {
            frontload: true,
            ..act
        }
    }

    fn names(resolved: &[ResolvedPlugin], plugin_id: &str) -> Vec<String> {
        resolved
            .iter()
            .find(|r| r.plugin_id == plugin_id)
            .map_or_default(|r| r.skills.names.clone())
    }

    fn frontloaded(resolved: &[ResolvedPlugin], plugin_id: &str) -> Vec<String> {
        resolved
            .iter()
            .find(|r| r.plugin_id == plugin_id)
            .map_or_default(|r| r.skills.frontloaded.clone())
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

    // ---- Workspace reach ----

    #[test]
    fn a_workspace_record_switches_a_global_one_off() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b"])],
            &[
                act("p1", None, Reach::Everyone, true),
                act("p1", None, Reach::Workspace, false),
            ],
        );
        assert!(
            resolved.is_empty(),
            "a workspace has to be able to keep a plugin out of the work done in it"
        );
    }

    #[test]
    fn a_workspace_record_switches_a_global_one_on() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                act("p1", None, Reach::Everyone, false),
                act("p1", None, Reach::Workspace, true),
            ],
        );
        assert_eq!(
            names(&resolved, "p1"),
            vec!["a"],
            "adding a skill nobody turned on globally is half of what a \
             workspace record is for"
        );
    }

    #[test]
    fn a_workspace_record_beats_a_personal_one() {
        let excluded = resolve(
            &[plugin("p1", &["a"])],
            &[
                act("p1", None, Reach::Personal, true),
                act("p1", None, Reach::Workspace, false),
            ],
        );
        assert!(
            excluded.is_empty(),
            "a personal record must not re-admit what the workspace excluded"
        );

        let added = resolve(
            &[plugin("p1", &["a"])],
            &[
                act("p1", None, Reach::Personal, false),
                act("p1", None, Reach::Workspace, true),
            ],
        );
        assert_eq!(
            names(&added, "p1"),
            vec!["a"],
            "workspace wins in both directions, not just off"
        );
    }

    #[test]
    fn a_workspace_skill_record_carves_one_skill_out_of_a_global_plugin() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b", "c"])],
            &[
                act("p1", None, Reach::Everyone, true),
                act("p1", Some("b"), Reach::Workspace, false),
            ],
        );
        assert_eq!(names(&resolved, "p1"), vec!["a", "c"]);
    }

    #[test]
    fn a_personal_skill_record_beats_a_workspace_plugin_record() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b"])],
            &[
                act("p1", None, Reach::Workspace, true),
                act("p1", Some("b"), Reach::Personal, false),
            ],
        );
        assert_eq!(
            names(&resolved, "p1"),
            vec!["a", "b"],
            "a blanket workspace record outranks a personal one however \
             specific it is: what the workspace turned on stays on"
        );
    }

    #[test]
    fn a_workspace_record_changes_nothing_where_it_does_not_apply() {
        // The same plugin resolved with no workspace records in play — what
        // somebody gets outside that workspace.
        let elsewhere = resolve(
            &[plugin("p1", &["a"])],
            &[act("p1", None, Reach::Everyone, true)],
        );
        assert_eq!(
            names(&elsewhere, "p1"),
            vec!["a"],
            "narrowing a plugin in one workspace must not follow somebody out of it"
        );
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

    #[test]
    fn a_row_reaches_by_whichever_subject_column_it_fills_in() {
        let row = |user_id, workspace_id: Option<&str>| crate::plugin::models::ActivationRow {
            plugin_id: "p1".to_string(),
            skill_name: None,
            user_id,
            workspace_id: workspace_id.map(str::to_string),
            enabled: true,
            always: false,
            frontload: false,
        };

        assert_eq!(reach_of(&row(None, None)), Reach::Everyone);
        assert_eq!(
            reach_of(&row(Some(uuid::Uuid::from_u128(1)), None)),
            Reach::Personal
        );
        assert_eq!(reach_of(&row(None, Some("ws1"))), Reach::Workspace);
    }

    // ---- Always ----

    #[test]
    fn a_global_always_record_outlasts_a_workspace_that_turns_it_off() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                always(act("p1", None, Reach::Everyone, true)),
                act("p1", None, Reach::Workspace, false),
            ],
        );
        assert_eq!(
            names(&resolved, "p1"),
            vec!["a"],
            "a skill marked always has to arrive in a workspace nobody told \
             about it, which is the whole of what the flag is for"
        );
    }

    #[test]
    fn a_personal_always_record_outlasts_a_workspace_skill_record() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                always(act("p1", None, Reach::Personal, true)),
                act("p1", Some("a"), Reach::Workspace, false),
            ],
        );
        assert_eq!(
            names(&resolved, "p1"),
            vec!["a"],
            "always outranks the scope order however specific the record \
             below it is"
        );
    }

    #[test]
    fn always_pins_an_off_record_too() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                always(act("p1", None, Reach::Everyone, false)),
                act("p1", None, Reach::Workspace, true),
            ],
        );
        assert!(
            resolved.is_empty(),
            "the flag pins whichever decision its record carries — an \
             administrator has to be able to keep a plugin out for good"
        );
    }

    #[test]
    fn among_always_records_the_scope_order_still_decides() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                always(act("p1", None, Reach::Everyone, true)),
                always(act("p1", None, Reach::Workspace, false)),
            ],
        );
        assert!(
            resolved.is_empty(),
            "the flag lifts a record out of the scope order only against \
             records that do not carry it"
        );
    }

    // ---- Frontload ----

    #[test]
    fn a_frontloading_record_names_the_skills_it_delivers() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b"])],
            &[frontloading(act("p1", None, Reach::Everyone, true))],
        );
        assert_eq!(names(&resolved, "p1"), vec!["a", "b"]);
        assert_eq!(
            frontloaded(&resolved, "p1"),
            vec!["a", "b"],
            "a plugin-wide record speaks for every skill on this axis as well"
        );
    }

    #[test]
    fn a_skill_nobody_delivers_is_never_frontloaded() {
        let resolved = resolve(
            &[plugin("p1", &["a", "b"])],
            &[
                frontloading(act("p1", None, Reach::Everyone, true)),
                act("p1", Some("b"), Reach::Personal, false),
            ],
        );
        assert_eq!(names(&resolved, "p1"), vec!["a"]);
        assert_eq!(
            frontloaded(&resolved, "p1"),
            vec!["a"],
            "frontloading a skill that was turned off would ask the sandbox \
             for a body it never cloned"
        );
    }

    #[test]
    fn the_record_that_decides_a_skill_decides_how_it_arrives() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                frontloading(act("p1", None, Reach::Everyone, true)),
                act("p1", None, Reach::Workspace, true),
            ],
        );
        assert_eq!(names(&resolved, "p1"), vec!["a"]);
        assert!(
            frontloaded(&resolved, "p1").is_empty(),
            "the workspace record won the skill, so its silence on \
             frontloading is the answer"
        );
    }

    #[test]
    fn an_always_record_carries_its_frontloading_past_the_scope_order() {
        let resolved = resolve(
            &[plugin("p1", &["a"])],
            &[
                always(frontloading(act("p1", None, Reach::Everyone, true))),
                act("p1", None, Reach::Workspace, true),
            ],
        );
        assert_eq!(
            frontloaded(&resolved, "p1"),
            vec!["a"],
            "the two flags travel together: the record that wins is the one \
             that says how the skill arrives"
        );
    }

    #[test]
    fn a_plugin_is_deselected_only_when_it_has_stored_skills_and_none_picked() {
        let registered: Vec<_> = ["picked", "dropped", "unread"]
            .into_iter()
            .map(registered_plugin)
            .collect();
        let skills: Vec<crate::plugin::models::PluginSkill> = ["picked", "dropped"]
            .into_iter()
            .map(|id| crate::plugin::models::PluginSkill {
                plugin_id: id.to_string(),
                name: "a".to_string(),
                description: String::new(),
                path: "skills/a/SKILL.md".to_string(),
            })
            .collect();
        let resolved = resolve(
            &[plugin("picked", &["a"]), plugin("dropped", &["a"])],
            &[
                act("picked", None, Reach::Everyone, true),
                act("dropped", None, Reach::Everyone, false),
            ],
        );

        let removed: Vec<&str> = deselected(&registered, &skills, &resolved)
            .into_iter()
            .map(|p| p.id.as_str())
            .collect();

        assert_eq!(
            removed,
            vec!["dropped"],
            "`unread` has no stored skills, so leaving it out says nothing \
             about whether anybody turned it off"
        );
    }

    fn registered_plugin(id: &str) -> crate::plugin::models::Plugin {
        crate::plugin::models::Plugin {
            id: id.to_string(),
            name: id.to_string(),
            git_url: format!("https://github.com/acme/{id}"),
            source: moatless_workspace_types::plugin::PluginSource::Manual,
            synced_version: None,
            synced_at: None,
            disabled_at: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn stored(plugin_id: &str, names: &[&str]) -> Vec<crate::plugin::models::PluginSkill> {
        names
            .iter()
            .map(|name| crate::plugin::models::PluginSkill {
                plugin_id: plugin_id.to_string(),
                name: (*name).to_string(),
                description: String::new(),
                path: format!("skills/{name}/SKILL.md"),
            })
            .collect()
    }

    fn workspace_row(plugin_id: &str, skill: &str) -> crate::plugin::models::ActivationRow {
        crate::plugin::models::ActivationRow {
            plugin_id: plugin_id.to_string(),
            skill_name: Some(skill.to_string()),
            user_id: None,
            workspace_id: Some("ws-1".to_string()),
            enabled: true,
            always: false,
            frontload: false,
        }
    }

    #[test]
    fn a_workspace_that_picks_another_plugin_still_gets_the_prompts_skills() {
        let registered = vec![
            registered_plugin("general"),
            registered_plugin(PROMPT_SKILLS_PLUGIN),
        ];
        let mut skills = stored("general", &["linear", "notion"]);
        skills.extend(stored(
            PROMPT_SKILLS_PLUGIN,
            &["artifact-design", "artifacts", "code-review", "pr-review"],
        ));

        let only_linear = resolve_rows(&registered, &skills, &[workspace_row("general", "linear")]);
        assert_eq!(names(&only_linear, "general"), vec!["linear"]);
        assert_eq!(
            names(&only_linear, PROMPT_SKILLS_PLUGIN),
            vec!["artifact-design", "artifacts"]
        );

        let with_review = resolve_rows(
            &registered,
            &skills,
            &[workspace_row(PROMPT_SKILLS_PLUGIN, "pr-review")],
        );
        assert_eq!(
            names(&with_review, PROMPT_SKILLS_PLUGIN),
            vec!["artifact-design", "artifacts", "pr-review"]
        );
    }
}
