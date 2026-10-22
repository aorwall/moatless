//! Which model and agent a conversation reports running.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::{debug, warn};

use super::scalar::non_empty;
use super::turn::turn_is_open;
use crate::AgentModelDefaults;
use crate::agent::catalog;
use crate::dao::Conversation;
use crate::providers;
use moatless_workspaces::workspace::models::Workspace;

/// The last resort of the model chain, and the only invented value in the row.
///
/// Deliberately not a plausible model name, so a sidebar never shows a
/// real-looking model nobody chose; [`report_placeholder`] logs an arrival with a
/// Turn.
const UNKNOWN_MODEL: &str = "unknown";

/// How many Tasks [`WARNED`] remembers before it forgets them all and each
/// warns once more.
const WARNED_CAPACITY: usize = 4096;

/// The Tasks this process has already warned about. One V2 projection resolves
/// the model once per run and a watched thread is rebuilt on every head move,
/// so without this one Task logs hundreds of lines a minute.
///
/// Process-global: a test that asserts the warning needs a Task id no other
/// test resolves.
static WARNED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// The provider instance of a conversation whose agent this build does not
/// know. No instance is published under it, so the thread reads as unavailable.
const UNKNOWN_INSTANCE: &str = "unknown";

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
    if let Some(model) = conversation
        .agent_profile
        .harness()
        .and_then(|harness| defaults.for_harness(harness))
    {
        return model.to_string();
    }
    // 4. The invention, logged so it is visible rather than silent — except
    //    for a conversation with no Turn, which has no model to find and is
    //    re-listed on every shell listing.
    if conversation.turn_number.is_some() {
        report_placeholder(conversation);
    }
    UNKNOWN_MODEL.to_string()
}

/// Logs a placeholder model at WARN once per Task per process, and at DEBUG
/// while the Turn is open.
///
/// An open Turn has no model until the agent's first text reply, since tool
/// calls carry none; a Task started without a model choice, as an adapter
/// starts one, waits there on every turn.
fn report_placeholder(conversation: &Conversation) {
    let task_id = &conversation.task_id;
    if turn_is_open(conversation) {
        debug!(%task_id, "No model reported yet for the open turn; reporting a placeholder");
        return;
    }
    let mut warned = WARNED.lock().unwrap_or_else(PoisonError::into_inner);
    if warned.contains(task_id) {
        return;
    }
    if warned.len() >= WARNED_CAPACITY {
        warned.clear();
    }
    warned.insert(task_id.clone());
    drop(warned);
    warn!(
        %task_id,
        agent_profile = %conversation.agent_profile,
        "No model could be resolved for the conversation; reporting a placeholder"
    );
}

/// The provider instance a model is resolved against.
///
/// The routing key, not part of the model name. An agent this build does not
/// know reports [`UNKNOWN_INSTANCE`], which T3 marks unavailable rather than
/// refusing to decode.
pub(super) fn provider_instance(conversation: &Conversation) -> &'static str {
    conversation
        .agent_profile
        .harness()
        .map_or(UNKNOWN_INSTANCE, providers::instance_for)
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
