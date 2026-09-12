//! Which model and agent a conversation reports running.

use serde_json::{Value, json};
use tracing::warn;

use super::scalar::non_empty;
use crate::AgentModelDefaults;
use crate::agent::catalog;
use crate::agent::catalog::UiRpcAgentKind;
use crate::dao::Conversation;
use crate::providers;
use crate::workspace::WorkspaceView as Workspace;

/// The last resort of the model chain, and the only invented value in the row.
///
/// Reached only by a conversation that has never reported a model *and* runs a
/// kind of agent the deployment configures no default for. Deliberately not a
/// plausible model name: a sidebar showing a real-looking model nobody chose is
/// the failure this chain exists to avoid. Every arrival here is logged.
const UNKNOWN_MODEL: &str = "unknown";

/// The model the conversation is running, on the provider instance that runs it.
///
/// The contract requires both halves, refuses an empty one, and has no decoding
/// default for either, so the chain has to end in an answer.
pub(super) fn model_selection(conversation: &Conversation, defaults: &AgentModelDefaults) -> Value {
    json!({
        "instanceId": provider_instance(conversation),
        "model": model_name(conversation, defaults)
    })
}

/// The model name, resolved through the four steps that always end in an answer.
pub(super) fn model_name(conversation: &Conversation, defaults: &AgentModelDefaults) -> String {
    // 1. What the newest turn's agent replies said they were produced by. Only
    //    the agent's own reply carries the identity of the model that ran.
    if let Some(model) = conversation.assistant_model.as_deref().and_then(non_empty) {
        return model;
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
    if let Some(model) = defaults.for_kind(agent_kind(conversation)) {
        return model.to_string();
    }
    // 4. The invention, logged so it is visible rather than silent.
    warn!(
        task_id = %conversation.task_id,
        agent_type = %conversation.agent_type,
        "No model could be resolved for the conversation; reporting a placeholder"
    );
    UNKNOWN_MODEL.to_string()
}

/// Which driver family backs this conversation's agent.
fn agent_kind(conversation: &Conversation) -> UiRpcAgentKind {
    catalog::find(conversation.agent_type.as_str())
        .map_or(UiRpcAgentKind::ClaudeCode, |entry| entry.kind)
}

/// The provider instance a model is resolved against.
///
/// The instance is the routing key, not part of the model name, and Moatless
/// has no provider-instance concept. T3's default instance for a driver is
/// named after the driver itself, and a conversation's agent kind already names
/// the driver. An instance a build does not recognize is marked unavailable at
/// runtime rather than refusing to decode, so this is the safe half of the pair.
pub(super) fn provider_instance(conversation: &Conversation) -> &'static str {
    match agent_kind(conversation) {
        UiRpcAgentKind::ClaudeCode => "claudeAgent",
        UiRpcAgentKind::Codex => "codex",
    }
}

/// The Workspace's default model, in the `ModelSelection` shape T3 sends back.
///
/// `Null` for a Workspace that pins no model, or that pins one this catalog no
/// longer publishes: a selection naming a slug absent from the provider list is
/// dropped by the client anyway, and substituting a model nobody chose is the
/// failure `providers::publish_model` exists to avoid. The effort rides along as
/// an option under the instance's own descriptor id, which is where
/// `commands::selected_effort` reads it back.
///
/// The effort gets the same treatment: `default_effort` is an open column no
/// write path validates, and a level the selected model does not publish is a
/// choice that cannot run, so one outside `providers::publishes_effort` is
/// dropped and the model runs at its own default.
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
