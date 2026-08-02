//! The Moatless agent catalog, as T3's provider list.
//!
//! # Why two instances and not five
//!
//! Moatless has five catalog agents; four of them are Claude over different
//! transports (`claude-code`, `claude-code-tui`, `claude-code-cli`,
//! `claude-code-fable`). T3's `instanceId` is a *routing key*: it is what a
//! thread is pinned to and what the composer resolves a model against. The
//! thread projection already derives `claudeAgent`/`codex` from a Task's agent
//! kind (`projection::provider_instance`), so publishing one instance per
//! catalog agent would rewrite `instanceId` on every thread already on screen
//! and put four near-identical rows in a picker where a person is choosing an
//! agent, not a transport.
//!
//! The transport is therefore not selectable from T3. It is chosen at Task
//! creation and is immutable afterwards, which is also why a model belonging to
//! the *other* instance cannot be honoured on an existing thread.
//!
//! # Why the published slugs and names are T3's, not ours
//!
//! T3 resolves a stored selection against the published list in three steps
//! (`packages/shared/src/model.ts:resolveSelectableModel`): exact slug, then
//! case-insensitive `name`, then the slug rewritten through
//! `MODEL_SLUG_ALIASES_BY_PROVIDER`. Nothing matching falls back to the
//! instance default with nothing on screen to say so
//! (`apps/web/src/modelSelection.ts:resolveAppModelSelectionForInstance`).
//!
//! Publishing our own ids would survive step one for a selection *we* stored —
//! but not for the slugs T3 itself seeds. `DEFAULT_MODEL_BY_PROVIDER` and
//! `DEFAULT_TEXT_GENERATION_MODEL_BY_PROVIDER` (`packages/contracts/src/model.ts`)
//! hand out `claude-sonnet-5` and `claude-haiku-4-5` as the client's own
//! defaults, and a selection carried over from a standalone T3 is in the same
//! vocabulary. A list published as `opus`/`sonnet`/`haiku` resolves none of
//! them: the alias table maps *towards* the canonical slugs, never away, so
//! step three cannot get back to ours either.
//!
//! So the canonical slugs go out and are translated back on the way in, and the
//! display names go out with them. T3's own Claude driver names these models
//! with their versions — "Claude Opus 5", not "Claude Sonnet"
//! (`apps/server/src/provider/Layers/ClaudeProvider.ts`) — and T3 renders
//! whatever `name` a provider sends, with no table of its own to fall back on.
//! Publishing the catalogue's version-free labels beside a versioned slug put
//! two different claims about the same model on one row.
//!
//! What the version in that name does *not* promise is a pinned model. The
//! catalogue id behind it is an alias (`opus`), which Claude Code resolves to
//! the current Opus at run time. The name follows the slug, and the slug is
//! T3's; if Anthropic moves what `opus` means, this table is what has to move
//! with it.
//!
//! No Codex alias rewrites any of our Codex ids, and the Moatless catalogue
//! already names them with versions, so that side needs no translation at all.
//!
//! # Why `default` is not published
//!
//! The catalogue's first entry for both agents is `default`, which names no
//! model: it means *whatever this agent picks*, is resolved inside the Sandbox
//! at run time, and is never written back. The thread projection already
//! refuses to report it for exactly that reason
//! (`projection::UNRESOLVED_MODEL_CHOICE`) — a row reading "Default" tells
//! nobody what ran. Offering it in the picker while refusing to display it is
//! the same contradiction seen from the other end, and T3 has no such concept
//! of its own: its default is `isDefault` on a real model.
//!
//! So the sentinel is dropped here and `isDefault` moves onto the model the
//! deployment actually falls back to — the same `AgentModelDefaults` the
//! projection resolves a model-less thread with. That is what makes the picker
//! and the thread agree: a conversation nobody chose a model for reports
//! `claude-opus-5`, and `claude-opus-5` is the row marked default.

use serde_json::{json, Value};

use crate::agent::catalog::AgentKind;
use crate::agent::schemas::AgentDescriptor;
use crate::ui_rpc::projection::UNRESOLVED_MODEL_CHOICE;
use crate::ui_rpc::AgentModelDefaults;

/// The two provider instances this surface publishes, in display order.
pub const CLAUDE_INSTANCE: &str = "claudeAgent";
pub const CODEX_INSTANCE: &str = "codex";

/// Which instance a catalog agent belongs to.
pub fn instance_for(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::ClaudeCode => CLAUDE_INSTANCE,
        AgentKind::Codex => CODEX_INSTANCE,
    }
}

/// The option-descriptor id an instance publishes its effort choice under.
///
/// The ids are T3's own, for the same reason the model slugs are: a selection
/// carried over from a standalone T3 is stored against `effort` for Claude and
/// `reasoningEffort` for Codex (`apps/server/src/provider/Layers/`), and one
/// published under any other id would leave that selection unread and silently
/// run the default.
pub fn effort_option_id(instance: &str) -> &'static str {
    if instance == CODEX_INSTANCE {
        "reasoningEffort"
    } else {
        "effort"
    }
}

/// The effort levels the Claude CLI's `--effort` accepts, weakest first. It
/// warns-and-ignores anything else, and does not vary them per model.
const CLAUDE_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// The effort levels a Codex model accepts beyond the base set, weakest first,
/// as that model's own `model/list` entry reports them.
///
/// Codex validates the level against the model and fails the whole turn on one
/// it does not know — `'minimal' is not supported with the 'gpt-5.6-sol' model`
/// — rather than falling back, so a row publishing a level its model rejects
/// hands out a choice that cannot run.
const CODEX_EFFORT_LEVELS: &[(&str, &[&str])] = &[
    (
        "gpt-5.6-sol",
        &["low", "medium", "high", "xhigh", "max", "ultra"],
    ),
    (
        "gpt-5.6-terra",
        &["low", "medium", "high", "xhigh", "max", "ultra"],
    ),
    ("gpt-5.6-luna", &["low", "medium", "high", "xhigh", "max"]),
];

/// What every Codex model accepts, and so what a model not named in
/// [`CODEX_EFFORT_LEVELS`] offers.
const CODEX_BASE_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh"];

/// The level that runs when nobody chooses one — what the Sandbox sends for
/// both agents. It is valid on every model published here, which is what lets
/// one constant serve them all.
const DEFAULT_EFFORT: &str = "high";

/// The effort levels a published model row offers, weakest first.
fn effort_levels(instance: &str, catalog_model: &str) -> &'static [&'static str] {
    if instance != CODEX_INSTANCE {
        return CLAUDE_EFFORT_LEVELS;
    }
    CODEX_EFFORT_LEVELS
        .iter()
        .find(|(model, _)| *model == catalog_model)
        .map_or(CODEX_BASE_EFFORT_LEVELS, |(_, levels)| *levels)
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
/// T3 reads the descriptor off the selected model's `capabilities` and sends
/// the chosen value back on `modelSelection.options`, where
/// `commands::selected_effort` reads it.
fn capabilities(instance: &str, catalog_model: &str) -> Value {
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
            "id": effort_option_id(instance),
            "label": "Reasoning",
            "type": "select",
            "options": options,
        }],
    })
}

/// `(catalog id, published slug, published name)` for the Claude instance.
///
/// The slugs and names are both T3's, copied from its own Claude driver
/// (`apps/server/src/provider/Layers/ClaudeProvider.ts`), so a model reads the
/// same here as it does in a standalone T3.
const CLAUDE_MODELS: &[(&str, &str, &str)] = &[
    ("opus", "claude-opus-5", "Claude Opus 5"),
    ("sonnet", "claude-sonnet-5", "Claude Sonnet 5"),
    ("haiku", "claude-haiku-4-5", "Claude Haiku 4.5"),
    ("fable", "claude-fable-5", "Claude Fable 5"),
];

/// The slug we publish for a catalog model id.
fn published_slug(instance: &str, catalog_model: &str) -> String {
    if instance == CLAUDE_INSTANCE {
        if let Some((_, slug, _)) = CLAUDE_MODELS.iter().find(|(id, _, _)| *id == catalog_model) {
            return (*slug).to_string();
        }
    }
    catalog_model.to_string()
}

/// The display name we publish for a catalog model id.
///
/// T3 renders this verbatim — it has no name table to fall back on — and also
/// matches a stored selection against it, so it is part of the contract rather
/// than decoration.
fn published_name(instance: &str, catalog_model: &str, catalog_label: &str) -> String {
    if instance == CLAUDE_INSTANCE {
        if let Some((_, _, name)) = CLAUDE_MODELS.iter().find(|(id, _, _)| *id == catalog_model) {
            return (*name).to_string();
        }
    }
    catalog_label.to_string()
}

/// The catalog model id behind a slug T3 sent back, or `None` when we never
/// published it.
///
/// `None` is a refusal, not a cue to fall back to the default: substituting a
/// model the person did not choose is the failure this whole translation exists
/// to avoid.
pub fn catalog_model_for(
    agents: &[AgentDescriptor],
    instance: &str,
    slug: &str,
) -> Option<String> {
    models_for(agents, instance)
        .into_iter()
        .find(|(id, _)| published_slug(instance, id) == slug)
        .map(|(id, _)| id)
}

/// Every `(catalog id, label)` an instance offers, deduplicated across the
/// catalog agents that share its kind, in catalog order.
///
/// The `default` sentinel is left out: it names no model, and publishing it
/// puts a row in the picker that cannot answer "what will run?".
fn models_for(agents: &[AgentDescriptor], instance: &str) -> Vec<(String, String)> {
    let mut models: Vec<(String, String)> = Vec::new();
    for agent in agents.iter().filter(|a| kind_of(a) == instance) {
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

/// The instance a descriptor belongs to, looked up through the catalog.
fn kind_of(agent: &AgentDescriptor) -> &'static str {
    crate::agent::catalog::find(&agent.id)
        .map_or(CLAUDE_INSTANCE, |entry| instance_for(entry.kind))
}

/// The catalog id of the model to mark `isDefault` for an instance.
///
/// The deployment's configured fallback decides it, because that is the value
/// the thread projection reports for a conversation nobody chose a model for —
/// so this is what keeps the picker and the thread saying the same thing. It is
/// matched in both vocabularies: the setting is free text consumed by the
/// Sandbox, and both `opus` and `claude-opus-5` are things an operator writes.
///
/// A deployment that configures nothing, or configures a model this catalog
/// cannot run, falls back to the first publishable model — T3 does the same
/// when no row is flagged, and doing it here at least keeps the flag truthful
/// about which row that is.
fn default_model_for(
    agents: &[AgentDescriptor],
    instance: &str,
    configured: Option<&str>,
) -> Option<String> {
    let models = models_for(agents, instance);
    let matched = configured.filter(|c| *c != UNRESOLVED_MODEL_CHOICE).and_then(|c| {
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
        .filter(|a| kind_of(a) == instance)
        .any(|a| a.installed)
}

/// The hint to show when nothing of this kind is usable.
fn missing_hint(agents: &[AgentDescriptor], instance: &str) -> Option<String> {
    agents
        .iter()
        .filter(|a| kind_of(a) == instance)
        .find_map(|a| a.missing_credential_hint.clone())
}

/// The provider list, in the shape `ServerProvider` decodes from.
///
/// `requiresNewThreadForModelChange` is deliberately left unset. T3 has that
/// flag for exactly the case where a thread cannot change providers, but it is
/// all-or-nothing per provider: setting it blocks *every* model change on a
/// started thread (`apps/web/src/components/ChatView.logic.ts:
/// getStartedThreadModelChangeBlockReason`), which would take per-turn model
/// choice with it. A cross-instance selection is refused at dispatch instead.
pub fn providers(
    agents: &[AgentDescriptor],
    defaults: &AgentModelDefaults,
    checked_at: &str,
) -> Vec<Value> {
    [
        (CLAUDE_INSTANCE, "Claude Code", AgentKind::ClaudeCode),
        (CODEX_INSTANCE, "Codex", AgentKind::Codex),
    ]
    .into_iter()
    .map(|(instance, display_name, kind)| {
        let is_installed = installed(agents, instance);
        let default_model = default_model_for(agents, instance, defaults.for_kind(kind));
        let models: Vec<Value> = models_for(agents, instance)
            .into_iter()
            .map(|(id, label)| {
                json!({
                    "slug": published_slug(instance, &id),
                    "name": published_name(instance, &id, &label),
                    "isCustom": false,
                    "isDefault": Some(&id) == default_model.as_ref(),
                    "capabilities": capabilities(instance, &id),
                })
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
            "showInteractionModeToggle": true,
            "availability": "available",
            "models": models,
            "slashCommands": [],
            "skills": [],
        });

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
            .iter()
            .map(|entry| AgentDescriptor {
                id: entry.id.to_string(),
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
    fn publishes_exactly_two_instances_in_order() {
        let list = published();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["instanceId"], CLAUDE_INSTANCE);
        assert_eq!(list[1]["instanceId"], CODEX_INSTANCE);
    }

    /// The regression that matters most: T3 rewrites `opus` to `claude-opus-5`
    /// before matching, so publishing `opus` silently runs the default.
    #[test]
    fn claude_models_are_published_under_t3s_canonical_names() {
        let list = published();
        assert_eq!(
            slugs(&list[0]),
            vec![
                "claude-opus-5",
                "claude-sonnet-5",
                "claude-haiku-4-5",
                "claude-fable-5"
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
                "Claude Opus 5",
                "Claude Sonnet 5",
                "Claude Haiku 4.5",
                "Claude Fable 5"
            ]
        );
    }

    /// The Moatless catalogue already names Codex models with their versions,
    /// so that side is published untranslated — including the names.
    #[test]
    fn codex_models_keep_the_catalogues_own_names() {
        let list = published();
        assert_eq!(names(&list[1])[0], "GPT-5.6 Sol");
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
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex-spark"
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
        assert_eq!(catalog_model_for(&agents, CODEX_INSTANCE, "gpt-4"), None);
    }

    /// The picker's default row and the model a chooseless thread reports are
    /// the same claim, so both come from the deployment's configured fallback.
    /// It is free text an operator writes, so either vocabulary has to land.
    #[test]
    fn the_configured_default_is_the_flagged_row_in_either_vocabulary() {
        for configured in ["opus", "claude-opus-5"] {
            let list = providers(
                &agents(true),
                &AgentModelDefaults {
                    claude_code: Some(configured.to_string()),
                    codex: None,
                },
                "2026-01-01T00:00:00.000Z",
            );
            let flagged: Vec<String> = list[0]["models"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["isDefault"] == json!(true))
                .map(|m| m["slug"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(flagged, vec!["claude-opus-5"], "configured {configured}");
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
            effort_descriptor(&list[0], "claude-opus-5")["id"],
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
            level_ids(effort_descriptor(&list[0], "claude-opus-5")),
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

    /// Every row's default has to be a level that row also offers, or leaving
    /// the picker alone picks something the turn will be refused for.
    #[test]
    fn exactly_one_level_per_row_is_the_default() {
        for provider in published() {
            for row in provider["models"].as_array().unwrap() {
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
    fn every_instance_offers_the_build_plan_toggle() {
        for provider in published() {
            assert_eq!(provider["showInteractionModeToggle"], json!(true));
        }
    }

    #[test]
    fn an_uninstalled_instance_is_disabled_and_carries_its_hint() {
        let list = providers(
            &agents(false),
            &AgentModelDefaults::default(),
            "2026-01-01T00:00:00.000Z",
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
}
