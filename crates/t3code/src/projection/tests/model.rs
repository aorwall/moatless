//! Which model a row reports, and the provider instance that goes with it.

use super::fixtures::*;
use super::*;

/// The chain has to end in an answer, because the contract refuses an empty one
/// and supplies no default of its own.
#[test]
fn the_model_chain_ends_in_an_answer() {
    let defaults = AgentModelDefaults {
        claude_code: Some("claude-opus-4-5".to_string()),
        codex: Some("gpt-5.6-sol".to_string()),
    };
    let mut row = conversation("task-a", "ws-a");

    assert_eq!(model_name(&row, &defaults), "claude-opus-4-5");

    // A catalog choice that names no model is not an answer: it resolves
    // inside the sandbox and is never written back.
    row.requested_model = Some("default".to_string());
    assert_eq!(model_name(&row, &defaults), "claude-opus-4-5");

    row.requested_model = Some("opus".to_string());
    assert_eq!(model_name(&row, &defaults), "opus");

    // Only the agent's own reply carries the identity of the model that ran.
    row.assistant_model = Some("claude-opus-4-5-20260115".to_string());
    assert_eq!(model_name(&row, &defaults), "claude-opus-4-5-20260115");

    row.agent_type = AgentType("codex".to_string());
    assert_eq!(
        model_name(&conversation("task-b", "ws-a"), &defaults),
        "claude-opus-4-5"
    );
    assert_eq!(model_name(&row, &defaults), "claude-opus-4-5-20260115");
}

#[test]
fn a_deployment_that_configures_no_default_still_reports_a_model() {
    let row = conversation("task-a", "ws-a");
    assert_eq!(model_name(&row, &AgentModelDefaults::default()), "unknown");
}

#[test]
fn the_provider_instance_follows_the_agent_kind() {
    let mut row = conversation("task-a", "ws-a");
    assert_eq!(provider_instance(&row), "claudeAgent");
    row.agent_type = AgentType("codex".to_string());
    assert_eq!(provider_instance(&row), "codex");
    // An agent this build does not know is not a reason to refuse a row.
    row.agent_type = AgentType("from-a-newer-backend".to_string());
    assert_eq!(provider_instance(&row), "claudeAgent");
}
