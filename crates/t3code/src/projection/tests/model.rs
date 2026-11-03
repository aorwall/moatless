//! Which model a row reports, and the provider instance that goes with it.

use super::fixtures::*;
use super::*;
use crate::tests::logs::CapturedLogs;
use moatless_tasks::task_listing::ListedAgent;

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

    row.agent_profile = AgentProfile::Codex.into();
    assert_eq!(
        model_name(&conversation("task-b", "ws-a"), &defaults),
        "claude-opus-4-5"
    );
    assert_eq!(model_name(&row, &defaults), "claude-opus-4-5-20260115");
}

/// A turn reports the API id, which is the slug the picker published, so the
/// model goes out as the agent said it ran.
#[test]
fn a_reported_model_id_is_the_slug_t3_knows() {
    let defaults = AgentModelDefaults::default();
    let mut row = conversation("task-a", "ws-a");

    row.assistant_model = Some("claude-haiku-5-5".to_string());
    assert_eq!(model_name(&row, &defaults), "claude-haiku-5-5");

    // A generation the catalog moved past still names the model that ran.
    row.assistant_model = Some("claude-haiku-4-5-20251001".to_string());
    assert_eq!(model_name(&row, &defaults), "claude-haiku-4-5-20251001");
}

#[test]
fn a_deployment_that_configures_no_default_still_reports_a_model() {
    let row = conversation("task-a", "ws-a");
    assert_eq!(model_name(&row, &AgentModelDefaults::default()), "unknown");
}

/// A conversation that never ran has no model to find, and every shell listing
/// re-reads it, so its placeholder is not a warning.
#[test]
fn only_a_conversation_with_a_turn_warns_about_its_placeholder() {
    let defaults = AgentModelDefaults::default();
    let mut row = conversation("task-never-ran", "ws-a");

    let (model, warnings) = CapturedLogs::warnings_from(|| model_name(&row, &defaults));
    assert_eq!(model, "unknown");
    assert_eq!(warnings, "");

    row.turn_number = Some(1);
    let (model, warnings) = CapturedLogs::warnings_from(|| model_name(&row, &defaults));
    assert_eq!(model, "unknown");
    assert!(
        warnings.contains("No model could be resolved"),
        "{warnings}"
    );
}

/// A V2 projection resolves the model once per run, and a watched thread is
/// rebuilt on every head move, so one Task warns once rather than each time.
#[test]
fn repeated_projections_of_a_model_less_conversation_warn_once() {
    let defaults = AgentModelDefaults::default();
    let mut row = conversation("task-warned-once", "ws-a");
    row.turn_number = Some(3);
    row.turn_status = Some("completed".to_string());

    let ((), warnings) = CapturedLogs::warnings_from(|| {
        for _ in 0..200 {
            assert_eq!(model_name(&row, &defaults), "unknown");
        }
    });
    assert_eq!(
        warnings.matches("No model could be resolved").count(),
        1,
        "{warnings}"
    );
}

/// Tool calls carry no model, so an open turn waits for the agent's first text
/// reply; a Task started without a model choice reports a placeholder until
/// then, which is not a warning.
#[test]
fn an_open_turn_without_a_reported_model_does_not_warn() {
    let defaults = AgentModelDefaults::default();
    let mut row = conversation("task-open-turn", "ws-a");
    row.turn_number = Some(1);
    row.turn_status = Some("active".to_string());

    let (model, warnings) = CapturedLogs::warnings_from(|| model_name(&row, &defaults));
    assert_eq!(model, "unknown");
    assert_eq!(warnings, "");
}

#[test]
fn the_provider_instance_follows_the_agent_kind() {
    let mut row = conversation("task-a", "ws-a");
    assert_eq!(provider_instance(&row), "claudeAgent");
    row.agent_profile = AgentProfile::Codex.into();
    assert_eq!(provider_instance(&row), "codex");
    row.agent_profile = AgentProfile::MiniSwe.into();
    assert_eq!(provider_instance(&row), "mini-swe");
    // An agent this build does not know lists, on no instance it could run on.
    row.agent_profile = "from-a-newer-backend".to_string().into();
    assert_eq!(provider_instance(&row), "unknown");
    assert!(matches!(row.agent_profile, ListedAgent::Unknown(_)));
}
