//! Which model and agent a conversation reports running.

use serde_json::{Value, json};
use tracing::warn;

use super::scalar::non_empty;
use crate::AgentModelDefaults;
use crate::agent::catalog;
use crate::dao::Conversation;
use crate::providers;
use crate::workspace::WorkspaceView as Workspace;
use moatless_types::agent::Harness;

/// The last resort of the model chain, and the only invented value in the row.
///
/// Deliberately not a plausible model name, so a sidebar never shows a
/// real-looking model nobody chose; every arrival with a Turn is logged.
const UNKNOWN_MODEL: &str = "unknown";

/// The model the conversation is running, on the provider instance that runs
/// it.
///
/// The contract requires both halves and refuses an empty one, so the chain
/// always ends in an answer.
pub(super) fn model_selection(conversation: &Conversation, defaults: &AgentModelDefaults) -> Value {
    json!({
        "instanceId": provider_instance(conversation),
        "model": model_name(conversation, defaults)
    })
}

/// The model name, resolved through the four steps that always end in an
/// answer.
pub(super) fn model_name(conversation: &Conversation, defaults: &AgentModelDefaults) -> String {
    // 1. What the newest turn's agent replies said they were produced by. Only
    //    the agent's own reply carries the identity of the model that ran, and
    //    it names it by API id, which is not always the published slug.
    if let Some(model) = conversation.assistant_model.as_deref().and_then(non_empty) {
        return providers::published_slug_for_reported(provider_instance(conversation), &model);
    }
    // 2. What was asked for when that turn started — skipping a catalog choice
    //    that names no model, which is what it actually resolves to.
    if let Some(model) = conversation
        .requested_model
        .as_deref()
        .and_then(non_empty)
        .filter(|model| model != catalog::UNRESOLVED_MODEL_CHOICE)
    {
        return model;
    }
    // 3. The deployment's default for this kind of agent. Optional
    //    configuration: a deployment that sets none falls through.
    if let Some(model) = defaults.for_harness(harness(conversation)) {
        return model.to_string();
    }
    // 4. The invention, logged so it is visible rather than silent — except
    //    for a conversation with no Turn, which has no model to find and is
    //    re-listed on every shell listing.
    if conversation.turn_number.is_none() {
        return UNKNOWN_MODEL.to_string();
    }
    warn!(
        task_id = %conversation.task_id,
        agent_profile = %conversation.agent_profile,
        "No model could be resolved for the conversation; reporting a placeholder"
    );
    UNKNOWN_MODEL.to_string()
}

/// Which driver family backs this conversation's agent.
fn harness(conversation: &Conversation) -> Harness {
    catalog::find(conversation.agent_profile.as_str())
        .map_or(Harness::ClaudeCode, |entry| entry.harness)
}

/// The provider instance a model is resolved against.
///
/// The routing key, not part of the model name. An instance a build does not
/// recognize is marked unavailable at runtime rather than refusing to decode.
pub(super) fn provider_instance(conversation: &Conversation) -> &'static str {
    match harness(conversation) {
        Harness::ClaudeCode => "claudeAgent",
        Harness::Codex => "codex",
    }
}

/// The Workspace's default model, in the `ModelSelection` shape T3 sends back.
///
/// `Null` when the Workspace pins no model or one this catalog no longer
/// publishes. `default_effort` is unvalidated at write time, so a level outside
/// `providers::publishes_effort` is dropped and the model runs at its own
/// default.
pub(super) fn default_model_selection(workspace: &Workspace) -> Value {
    let Some(catalog_model) = workspace.default_model.as_deref() else {
        return Value::Null;
    };
    let Some((instance, slug)) = providers::publish_model(catalog_model) else {
        return Value::Null;
    };
    let mut selection = json!({ "instanceId": instance, "model": slug });
    if let Some(effort) = workspace
        .default_effort
        .as_deref()
        .map(str::trim)
        .filter(|level| providers::publishes_effort(instance, catalog_model, level))
    {
        selection["options"] = json!([{
            "id": providers::effort_option_id(instance),
            "value": effort,
        }]);
    }
    selection
}
