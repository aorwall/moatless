//! The Moatless agent catalog, as T3's provider list.
//!
//! Each harness has one provider instance; catalog transport variants share it.
//!
//! Publishes T3's canonical slugs and names (its alias table maps only towards
//! them); the `default` sentinel, naming no model, is never published.

use moatless_types::model::{
    ANTIGRAVITY_EFFORT_LEVELS, CLAUDE_EFFORT_LEVELS, PI_THINKING_LEVELS, claude_model_id,
    codex_effort_levels,
};
use serde_json::{Value, json};

use crate::AgentModelDefaults;
use crate::agent::catalog::{UNRESOLVED_MODEL_CHOICE, catalog};
use crate::agent::schemas::AgentDescriptor;
use moatless_types::agent::{AgentProfile, Harness, split_pi_model};

/// The Claude provider instance id.
pub const CLAUDE_INSTANCE: &str = "claudeAgent";
pub const CODEX_INSTANCE: &str = "codex";
pub const PI_INSTANCE: &str = "pi";
pub const MINI_SWE_INSTANCE: &str = "mini-swe";
/// Antigravity, which runs on the ACP harness: the instance names the agent a
/// person picks, not the protocol, so it is also T3's own `antigravity` driver.
pub const ANTIGRAVITY_INSTANCE: &str = "antigravity";
/// The id Antigravity was published under before; a stale client still sends it.
const LEGACY_ACP_INSTANCE: &str = "acp";
/// T3's `ANTIGRAVITY_DEFAULT_MODEL`: "whichever model the session runs".
pub const ANTIGRAVITY_DEFAULT_ALIAS: &str = "antigravity-default";

/// Everything T3 shows for one harness. [`presentation`] is the only place a
/// harness maps to T3's names, so a new harness does not build until it is
/// given every field here.
pub struct Presentation {
    pub instance: &'static str,
    pub display_name: &'static str,
    /// T3's `UsageProviderKind`, or `None` for a harness the usage page has no
    /// kind for, whose turns it leaves out.
    pub usage_kind: Option<&'static str>,
    /// `None` publishes no effort picker.
    pub effort: Option<EffortPicker>,
    pub interaction_mode_toggle: bool,
    /// The client reserves the context meter's space only where this is true.
    pub reports_context_window: bool,
    /// The slug T3's driver starts a draft on, published on the default model
    /// and read back as "no model chosen".
    pub default_alias: Option<&'static str>,
}

/// The effort choice a harness's models publish.
pub struct EffortPicker {
    /// The option-descriptor id T3 reads the choice from. Publishing under any
    /// other id leaves a carried-over selection unread and silently runs the
    /// default.
    pub option_id: &'static str,
    /// The levels a catalog model offers, weakest first.
    pub levels: fn(&str) -> &'static [&'static str],
}

/// How T3 shows `harness`.
pub fn presentation(harness: Harness) -> Presentation {
    match harness {
        Harness::ClaudeCode => Presentation {
            instance: CLAUDE_INSTANCE,
            display_name: "Claude Code",
            usage_kind: Some("claude"),
            effort: Some(EffortPicker {
                option_id: "effort",
                levels: |_| CLAUDE_EFFORT_LEVELS,
            }),
            interaction_mode_toggle: true,
            reports_context_window: true,
            default_alias: None,
        },
        Harness::Codex => Presentation {
            instance: CODEX_INSTANCE,
            display_name: "Codex",
            usage_kind: Some("codex"),
            effort: Some(EffortPicker {
                option_id: "reasoningEffort",
                levels: codex_effort_levels,
            }),
            interaction_mode_toggle: true,
            reports_context_window: true,
            default_alias: None,
        },
        Harness::Pi => Presentation {
            instance: PI_INSTANCE,
            display_name: "Pi",
            usage_kind: Some("pi"),
            effort: Some(EffortPicker {
                option_id: "thinking",
                levels: |_| PI_THINKING_LEVELS,
            }),
            interaction_mode_toggle: true,
            reports_context_window: true,
            default_alias: None,
        },
        // mini-swe-agent's harness sends no `context-window.updated`.
        Harness::MiniSwe => Presentation {
            instance: MINI_SWE_INSTANCE,
            display_name: "mini-swe-agent",
            usage_kind: None,
            effort: None,
            interaction_mode_toggle: false,
            reports_context_window: false,
            default_alias: None,
        },
        Harness::Acp => Presentation {
            instance: ANTIGRAVITY_INSTANCE,
            display_name: "Antigravity",
            usage_kind: Some("antigravity"),
            effort: Some(EffortPicker {
                option_id: "effort",
                levels: |_| ANTIGRAVITY_EFFORT_LEVELS,
            }),
            interaction_mode_toggle: true,
            reports_context_window: true,
            default_alias: Some(ANTIGRAVITY_DEFAULT_ALIAS),
        },
    }
}

/// Which instance a catalog agent belongs to.
pub fn instance_for(harness: Harness) -> &'static str {
    presentation(harness).instance
}

/// The harness an instance names; `None` for one this surface does not publish.
pub fn harness_for(instance: &str) -> Option<Harness> {
    if instance == LEGACY_ACP_INSTANCE {
        return Some(Harness::Acp);
    }
    Harness::ALL
        .iter()
        .copied()
        .find(|harness| presentation(*harness).instance == instance)
}

/// T3's usage kind for a Task's agent profile; see [`Presentation::usage_kind`].
pub fn usage_kind(profile: AgentProfile) -> Option<&'static str> {
    presentation(profile.harness()).usage_kind
}

fn effort_picker(instance: &str) -> Option<EffortPicker> {
    harness_for(instance).and_then(|harness| presentation(harness).effort)
}

/// The option-descriptor id an instance publishes its effort choice under; see
/// [`EffortPicker::option_id`]. `None` for an instance with no effort picker.
pub fn effort_option_id(instance: &str) -> Option<&'static str> {
    effort_picker(instance).map(|picker| picker.option_id)
}

/// The level that runs when nobody chooses one — what the Sandbox sends for
/// both agents. It is valid on every model published here, which is what lets
/// one constant serve them all.
const DEFAULT_EFFORT: &str = "high";

/// The effort levels a published model row offers, weakest first.
fn effort_levels(instance: &str, catalog_model: &str) -> &'static [&'static str] {
    effort_picker(instance).map_or(&[], |picker| (picker.levels)(catalog_model))
}

/// Whether a published model row offers `level`.
///
/// See [`codex_effort_levels`]: a rejected level fails the turn rather than
/// falling back, so a stored default naming one is dropped, not offered.
pub fn publishes_effort(instance: &str, catalog_model: &str, level: &str) -> bool {
    effort_levels(instance, catalog_model).contains(&level)
}

fn effort_label(level: &str) -> &str {
    match level {
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra High",
        "max" => "Max",
        "ultra" => "Ultra",
        other => other,
    }
}

/// The model capabilities a row publishes — today, the effort picker.
///
/// T3 sends the chosen value back on `modelSelection.options`.
fn capabilities(instance: &str, catalog_model: &str) -> Value {
    let Some(option_id) = effort_option_id(instance) else {
        return json!({"optionDescriptors": []});
    };
    let options: Vec<Value> = effort_levels(instance, catalog_model)
        .iter()
        .map(|level| {
            let mut option = json!({ "id": level, "label": effort_label(level) });
            if *level == DEFAULT_EFFORT {
                option["isDefault"] = json!(true);
            }
            option
        })
        .collect();

    json!({
        "optionDescriptors": [{
            "id": option_id,
            "label": "Reasoning",
            "type": "select",
            "options": options,
        }],
    })
}

/// The slug we publish for a catalog model id.
///
/// A Claude model goes out under its API id, which is also the slug T3's own
/// driver publishes. Do not add a model whose id T3 does not use as a slug: a
/// turn reports the API id, and T3 silently runs its default for a selection
/// it matches against nothing.
fn published_slug(instance: &str, catalog_model: &str) -> String {
    if instance == CLAUDE_INSTANCE {
        if let Some(id) = claude_model_id(catalog_model) {
            return id.to_string();
        }
    }
    catalog_model.to_string()
}

/// The catalog model id behind a slug T3 sent back, or `None` when we never
/// published it.
///
/// `None` is a refusal, not a fallback cue — substituting an unchosen model is
/// exactly the failure this translation exists to avoid.
pub fn catalog_model_for(agents: &[AgentDescriptor], instance: &str, slug: &str) -> Option<String> {
    models_for(agents, instance)
        .into_iter()
        .find(|(id, _)| published_slug(instance, id) == slug)
        .map(|(id, _)| id)
}

/// `(instance id, published slug)` for a catalog model id, or `None` when no
/// instance publishes one.
///
/// Resolves against the static catalog, not a runtime `AgentDescriptor` list,
/// since the project projection runs without one; a model missing a credential
/// is still published here.
pub fn publish_model(catalog_model: &str) -> Option<(&'static str, String)> {
    if catalog_model == UNRESOLVED_MODEL_CHOICE {
        return None;
    }
    catalog()
        .find(|agent| agent.models.iter().any(|m| m.id == catalog_model))
        .map(|agent| {
            let instance = instance_for(agent.harness());
            (instance, published_slug(instance, catalog_model))
        })
        // Pi runs models the hub lists beyond the static catalog; the vendor
        // prefix is what `validate_agent_model` requires of one.
        .or_else(|| split_pi_model(catalog_model).map(|_| (PI_INSTANCE, catalog_model.to_string())))
}

/// Every `(catalog id, label)` an instance offers, deduplicated across the
/// catalog agents that share its kind, in catalog order.
///
/// The `default` sentinel is left out: it names no model.
fn models_for(agents: &[AgentDescriptor], instance: &str) -> Vec<(String, String)> {
    let mut models: Vec<(String, String)> = Vec::new();
    for agent in agents.iter().filter(|a| kind_of(a) == Some(instance)) {
        for model in &agent.models {
            if model.id == UNRESOLVED_MODEL_CHOICE {
                continue;
            }
            if !models.iter().any(|(id, _)| *id == model.id) {
                models.push((model.id.clone(), model.label.clone()));
            }
        }
    }
    models
}

/// The instance a descriptor belongs to, looked up through the catalog; `None`
/// for an id the catalog does not know.
pub(crate) fn kind_of(agent: &AgentDescriptor) -> Option<&'static str> {
    crate::agent::catalog::find(&agent.id).map(|entry| instance_for(entry.harness()))
}

/// The catalog id of the model to mark `isDefault` for an instance.
///
/// The deployment's configured fallback decides it, matched against both `opus`
/// and `claude-opus-5-5` since the setting is free text; configuring nothing, or
/// an unrunnable model, falls back to the first publishable one.
fn default_model_for(
    agents: &[AgentDescriptor],
    instance: &str,
    configured: Option<&str>,
) -> Option<String> {
    let models = models_for(agents, instance);
    let matched = configured
        .filter(|c| *c != UNRESOLVED_MODEL_CHOICE)
        .and_then(|c| {
            models
                .iter()
                .find(|(id, _)| id == c || published_slug(instance, id) == c)
                .map(|(id, _)| id.clone())
        });
    matched.or_else(|| models.first().map(|(id, _)| id.clone()))
}

/// Whether any catalog agent of this kind has a usable credential.
fn installed(agents: &[AgentDescriptor], instance: &str) -> bool {
    agents
        .iter()
        .filter(|a| kind_of(a) == Some(instance))
        .any(|a| a.installed)
}

/// The hint to show when nothing of this kind is usable.
fn missing_hint(agents: &[AgentDescriptor], instance: &str) -> Option<String> {
    agents
        .iter()
        .filter(|a| kind_of(a) == Some(instance))
        .find_map(|a| a.missing_credential_hint.clone())
}

/// The provider list, in the shape `ServerProvider` decodes from.
///
/// `requiresNewThreadForModelChange` stays unset — a cross-instance selection
/// switches the thread's harness at dispatch; `workspace_snapshots` publish on every
/// instance, since a skill reaches whichever agent runs.
pub fn providers(
    agents: &[AgentDescriptor],
    defaults: &AgentModelDefaults,
    checked_at: &str,
    workspace_snapshots: &[Value],
) -> Vec<Value> {
    Harness::ALL
        .iter()
        .copied()
        .map(|harness| {
            let Presentation {
                instance,
                display_name,
                interaction_mode_toggle,
                reports_context_window,
                default_alias,
                ..
            } = presentation(harness);
            let is_installed = installed(agents, instance);
            let default_model = default_model_for(agents, instance, defaults.for_harness(harness));
            let models: Vec<Value> = models_for(agents, instance)
                .into_iter()
                .map(|(id, label)| {
                    let is_default = Some(&id) == default_model.as_ref();
                    let mut row = json!({
                        "slug": published_slug(instance, &id),
                        // T3 renders this verbatim and matches a stored selection against it.
                        "name": label,
                        "isCustom": false,
                        "isDefault": is_default,
                        "capabilities": capabilities(instance, &id),
                    });
                    if let Some(alias) = default_alias.filter(|_| is_default) {
                        row["aliases"] = json!([alias]);
                    }
                    row
                })
                .collect();

            let mut provider = json!({
                "instanceId": instance,
                "driver": instance,
                "displayName": display_name,
                "enabled": true,
                "installed": is_installed,
                "version": Value::Null,
                "status": if is_installed { "ready" } else { "disabled" },
                "auth": {
                    "status": if is_installed { "authenticated" } else { "unauthenticated" },
                },
                "checkedAt": checked_at,
                "showInteractionModeToggle": interaction_mode_toggle,
                // Both harnesses resume a branched transcript, which is the whole of
                // what a history-only rewind needs. The file-restoring half is
                // refused in `commands::dispatch` rather than turned off here: this
                // flag is per provider and would take both with it.
                "supportsConversationRollback": true,
                "reportsContextWindow": reports_context_window,
                "availability": "available",
                "models": models,
                "slashCommands": [],
                "skills": [],
            });

            // `workspaceSnapshots` is `optionalKey` in the contract, so a
            // connection that has resolved no `cwd` yet leaves it out.
            if !workspace_snapshots.is_empty() {
                provider["workspaceSnapshots"] = json!(workspace_snapshots);
            }

            if !is_installed {
                if let Some(hint) = missing_hint(agents, instance) {
                    provider["message"] = json!(hint);
                }
            }
            provider
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::catalog;
    use crate::agent::schemas::AgentModel;

    /// The full catalog with every agent installed.
    fn agents(installed: bool) -> Vec<AgentDescriptor> {
        catalog::catalog()
            .map(|entry| AgentDescriptor {
                id: entry.id().to_string(),
                label: entry.label.to_string(),
                installed,
                default_model: entry.default_model.to_string(),
                models: entry
                    .models
                    .iter()
                    .map(|m| AgentModel {
                        id: m.id.to_string(),
                        label: m.label.to_string(),
                    })
                    .collect(),
                missing_credential_hint: if installed {
                    None
                } else {
                    Some(entry.missing_credential_hint.to_string())
                },
            })
            .collect()
    }

    /// The list as a deployment that configures no defaults publishes it.
    fn published() -> Vec<Value> {
        providers(
            &agents(true),
            &AgentModelDefaults::default(),
            "2026-01-01T00:00:00.000Z",
            &[],
        )
    }

    fn slugs(provider: &Value) -> Vec<String> {
        field(provider, "slug")
    }

    fn names(provider: &Value) -> Vec<String> {
        field(provider, "name")
    }

    fn field(provider: &Value, key: &str) -> Vec<String> {
        provider["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m[key].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn a_hub_model_outside_the_catalog_publishes_on_pi() {
        assert_eq!(
            publish_model("anthropic/claude-opus-4-8"),
            Some((PI_INSTANCE, "anthropic/claude-opus-4-8".to_string()))
        );
        assert_eq!(
            publish_model("openrouter/mistralai/mistral-large-4-0"),
            Some((
                PI_INSTANCE,
                "openrouter/mistralai/mistral-large-4-0".to_string()
            ))
        );
        assert_eq!(publish_model("openai/"), None);
        assert_eq!(publish_model("claude-opus-4-8-unknown"), None);
    }

    #[test]
    fn every_harness_is_published_under_an_instance_that_maps_back_to_it() {
        let list = published();
        assert_eq!(list.len(), Harness::ALL.len());
        for (provider, harness) in list.iter().zip(Harness::ALL) {
            let instance = provider["instanceId"].as_str().unwrap();
            assert_eq!(harness_for(instance), Some(*harness), "{instance}");
        }
    }

    #[test]
    fn publishes_harness_instances_in_order() {
        let list = published();
        assert_eq!(list.len(), 5);
        assert_eq!(list[0]["instanceId"], CLAUDE_INSTANCE);
        assert_eq!(list[1]["instanceId"], CODEX_INSTANCE);
        assert_eq!(list[2]["instanceId"], "pi");
    }

    /// The regression that matters most: T3 rewrites `opus` to `claude-opus-5-5`
    /// before matching, so publishing `opus` silently runs the default.
    #[test]
    fn claude_models_are_published_under_t3s_canonical_names() {
        let list = published();
        assert_eq!(
            slugs(&list[0]),
            vec![
                "claude-opus-5-5",
                "claude-sonnet-5-5",
                "claude-haiku-5-5",
                "claude-fable-5-2"
            ]
        );
    }

    /// T3 renders whatever `name` a provider sends and has no table of its own
    /// to fall back on, so a version-free label beside a versioned slug is two
    /// claims about one model. These are the names T3's own Claude driver uses.
    #[test]
    fn claude_models_are_named_the_way_t3_names_them() {
        let list = published();
        assert_eq!(
            names(&list[0]),
            vec![
                "Claude Opus 5.5",
                "Claude Sonnet 5.5",
                "Claude Haiku 5.5",
                "Claude Fable 5.2"
            ]
        );
    }

    /// The Moatless catalogue already names Codex models with their versions,
    /// so that side is published untranslated — including the names.
    #[test]
    fn codex_models_keep_the_catalogues_own_names() {
        let list = published();
        assert_eq!(
            names(&list[1]),
            vec![
                "GPT-6.2 Sol",
                "GPT-6.2 Astra",
                "GPT-6.1 Terra",
                "GPT-6.1 Sol",
                "GPT-6 Astra",
                "GPT-6 Sol",
                "GPT-6 Luna",
                "GPT-5.6 Sol",
                "GPT-5.6 Terra",
                "GPT-5.6 Luna",
                "GPT-5.5"
            ]
        );
    }

    /// Every name is a matching key in `resolveSelectableModel`'s second step,
    /// so a duplicate inside one instance makes a stored selection ambiguous.
    #[test]
    fn names_are_unique_within_an_instance() {
        for provider in published() {
            let mut seen = names(&provider);
            let before = seen.len();
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), before, "{}", provider["instanceId"]);
        }
    }

    /// No Codex alias rewrites any of our ids, so they go out unchanged.
    #[test]
    fn codex_models_are_published_unchanged() {
        let list = published();
        assert_eq!(
            slugs(&list[1]),
            vec![
                "gpt-6.2-sol",
                "gpt-6.2-astra",
                "gpt-6.1-terra",
                "gpt-6.1-sol",
                "gpt-6-astra",
                "gpt-6-sol",
                "gpt-6-luna",
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5"
            ]
        );
    }

    #[test]
    fn every_published_slug_translates_back_to_a_catalog_id() {
        let agents = agents(true);
        let list = providers(
            &agents,
            &AgentModelDefaults::default(),
            "2026-01-01T00:00:00.000Z",
            &[],
        );
        for provider in &list {
            let instance = provider["instanceId"].as_str().unwrap();
            for slug in slugs(provider) {
                let resolved = catalog_model_for(&agents, instance, &slug);
                assert!(
                    resolved.is_some(),
                    "{instance}/{slug} did not translate back"
                );
                let resolved = resolved.unwrap();
                assert_eq!(
                    published_slug(instance, &resolved),
                    slug,
                    "{instance}/{slug} did not round-trip"
                );
            }
        }
    }

    #[test]
    fn an_unpublished_slug_is_refused_rather_than_defaulted() {
        let agents = agents(true);
        assert_eq!(
            catalog_model_for(&agents, CLAUDE_INSTANCE, "claude-opus-4-8"),
            None
        );
        assert_eq!(catalog_model_for(&agents, CLAUDE_INSTANCE, "opus"), None);
        assert_eq!(catalog_model_for(&agents, CLAUDE_INSTANCE, "fable"), None);
        // A generation the catalog moved past, as a stored selection names it.
        assert_eq!(
            catalog_model_for(&agents, CLAUDE_INSTANCE, "claude-haiku-4-5"),
            None
        );
        assert_eq!(catalog_model_for(&agents, CODEX_INSTANCE, "gpt-4"), None);
    }

    /// The picker's default row and the model a chooseless thread reports are
    /// the same claim, so both come from the deployment's configured fallback.
    /// It is free text an operator writes, so either vocabulary has to land.
    #[test]
    fn the_configured_default_is_the_flagged_row_in_either_vocabulary() {
        for configured in ["opus", "claude-opus-5-5"] {
            let list = providers(
                &agents(true),
                &AgentModelDefaults {
                    claude_code: Some(configured.to_string()),
                    codex: None,
                },
                "2026-01-01T00:00:00.000Z",
                &[],
            );
            let flagged: Vec<String> = list[0]["models"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["isDefault"] == json!(true))
                .map(|m| m["slug"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(flagged, vec!["claude-opus-5-5"], "configured {configured}");
        }
    }

    /// `default` names no model, and the thread projection already refuses to
    /// report it. Offering it in the picker is the same contradiction from the
    /// other end: nothing on that row answers "what will run?".
    #[test]
    fn the_choice_that_names_no_model_is_not_published() {
        for provider in published() {
            assert!(
                !slugs(&provider).contains(&"default".to_string()),
                "{}",
                provider["instanceId"]
            );
        }
    }

    #[test]
    fn exactly_one_model_per_instance_is_the_default() {
        let list = published();
        for provider in &list {
            let defaults = provider["models"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["isDefault"] == json!(true))
                .count();
            assert_eq!(defaults, 1, "{}", provider["instanceId"]);
        }
    }

    /// The effort descriptor as one model row publishes it.
    fn effort_descriptor<'a>(provider: &'a Value, slug: &str) -> &'a Value {
        let row = provider["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["slug"] == json!(slug))
            .unwrap_or_else(|| panic!("{} publishes no {slug}", provider["instanceId"]));
        let descriptors = row["capabilities"]["optionDescriptors"].as_array().unwrap();
        assert_eq!(descriptors.len(), 1, "{slug}");
        &descriptors[0]
    }

    fn level_ids(descriptor: &Value) -> Vec<String> {
        descriptor["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// The ids are the ones T3 stores a carried-over selection under, so a
    /// rename here leaves that selection unread and silently runs the default.
    #[test]
    fn each_instance_publishes_its_effort_picker_under_t3s_own_id() {
        let list = published();
        assert_eq!(
            effort_descriptor(&list[0], "claude-opus-5-5")["id"],
            json!("effort")
        );
        assert_eq!(
            effort_descriptor(&list[1], "gpt-5.6-sol")["id"],
            json!("reasoningEffort")
        );
    }

    /// The levels are what the agent behind the row accepts; Codex fails the
    /// whole turn on one its model does not know.
    #[test]
    fn the_published_levels_are_the_ones_the_agent_accepts() {
        let list = published();
        assert_eq!(
            level_ids(effort_descriptor(&list[0], "claude-opus-5-5")),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            level_ids(effort_descriptor(&list[1], "gpt-5.6-sol")),
            vec!["low", "medium", "high", "xhigh", "max", "ultra"]
        );
        assert_eq!(
            level_ids(effort_descriptor(&list[1], "gpt-5.6-luna")),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            level_ids(effort_descriptor(&list[1], "gpt-5.5")),
            vec!["low", "medium", "high", "xhigh"]
        );
    }

    #[test]
    fn antigravity_is_published_as_t3s_driver_with_its_default_alias() {
        let agents = vec![AgentDescriptor {
            id: "antigravity".into(),
            label: "Antigravity".into(),
            installed: true,
            default_model: "gemini-3-flash".into(),
            models: vec![
                moatless_api_schemas::agent::AgentModel {
                    id: "gemini-3-flash".into(),
                    label: "Gemini 3 Flash".into(),
                },
                moatless_api_schemas::agent::AgentModel {
                    id: "gemini-4-argon-high".into(),
                    label: "Gemini 4 Argon".into(),
                },
            ],
            missing_credential_hint: None,
        }];
        let list = providers(&agents, &AgentModelDefaults::default(), "now", &[]);
        let provider = list
            .iter()
            .find(|p| p["instanceId"] == "antigravity")
            .unwrap();
        assert_eq!(provider["driver"], "antigravity");
        assert_eq!(
            provider["models"][0]["aliases"],
            json!([ANTIGRAVITY_DEFAULT_ALIAS])
        );
        assert!(provider["models"][1].get("aliases").is_none());
        assert_eq!(harness_for("acp"), Some(Harness::Acp));
    }

    /// Every row's default has to be a level that row also offers, or leaving
    /// the picker alone picks something the turn will be refused for.
    #[test]
    fn exactly_one_level_per_row_is_the_default() {
        for provider in published() {
            for row in provider["models"].as_array().unwrap() {
                if provider["instanceId"] == MINI_SWE_INSTANCE {
                    assert_eq!(row["capabilities"]["optionDescriptors"], json!([]));
                    continue;
                }
                let slug = row["slug"].as_str().unwrap();
                let defaults: Vec<String> = effort_descriptor(&provider, slug)["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|o| o["isDefault"] == json!(true))
                    .map(|o| o["id"].as_str().unwrap().to_string())
                    .collect();
                assert_eq!(defaults, vec![DEFAULT_EFFORT], "{slug}");
            }
        }
    }

    /// The Claude CLI takes the same levels whatever model it runs, so a row
    /// carrying different ones would be inventing a distinction.
    #[test]
    fn every_claude_row_carries_the_same_capabilities() {
        let list = published();
        let rows = list[0]["models"].as_array().unwrap();
        for row in rows {
            assert_eq!(
                row["capabilities"], rows[0]["capabilities"],
                "{}",
                row["slug"]
            );
        }
    }

    #[test]
    fn only_harnesses_with_plan_mode_offer_the_toggle() {
        for provider in published() {
            assert_eq!(
                provider["showInteractionModeToggle"],
                json!(provider["instanceId"] != MINI_SWE_INSTANCE)
            );
        }
    }

    #[test]
    fn an_uninstalled_instance_is_disabled_and_carries_its_hint() {
        let list = providers(
            &agents(false),
            &AgentModelDefaults::default(),
            "2026-01-01T00:00:00.000Z",
            &[],
        );
        for provider in &list {
            assert_eq!(provider["installed"], json!(false));
            assert_eq!(provider["status"], json!("disabled"));
            assert_eq!(provider["auth"]["status"], json!("unauthenticated"));
            assert!(provider["message"].is_string());
        }
    }

    #[test]
    fn an_installed_instance_is_ready_and_carries_no_hint() {
        let list = published();
        for provider in &list {
            assert_eq!(provider["status"], json!("ready"));
            assert_eq!(provider["auth"]["status"], json!("authenticated"));
            assert!(provider["message"].is_null());
        }
    }

    #[test]
    fn a_connection_that_has_resolved_no_cwd_omits_workspace_snapshots() {
        for provider in &published() {
            assert!(provider.get("workspaceSnapshots").is_none());
        }
    }

    /// A skill is a file in the repository, so it reaches whichever agent the
    /// Task runs. Publishing the snapshot on only the instance whose composer
    /// asked would empty the menu on switching provider.
    #[test]
    fn a_resolved_cwd_is_published_on_every_instance() {
        let snapshot = json!({
            "cwd": "/moatless/workspaces/ws-1",
            "checkedAt": "2026-01-01T00:00:00.000Z",
            "slashCommands": [],
            "skills": [],
        });
        let list = providers(
            &agents(true),
            &AgentModelDefaults::default(),
            "2026-01-01T00:00:00.000Z",
            std::slice::from_ref(&snapshot),
        );

        assert_eq!(list.len(), 5);
        for provider in &list {
            assert_eq!(provider["workspaceSnapshots"], json!([snapshot]));
        }
    }
}
