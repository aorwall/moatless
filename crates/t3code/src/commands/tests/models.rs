//! The model, effort and interaction mode a turn picks, checked against what
//! this server published for the thread's agent.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn a_published_slug_reaches_the_sink_as_a_catalog_id() {
    let (_, calls) = run(turn(Some(("claudeAgent", "claude-sonnet-5")))).await;
    assert_eq!(
        calls,
        vec![Call::SendTurn {
            task_id: "task-1".to_string(),
            message_uuid: "msg-1".to_string(),
            model: Some("sonnet".to_string()),
            agent_mode: Some("edit".to_string()),
            effort: None,
            attachments: vec![],
            skills: vec![],
        }]
    );
}

/// A task's harness is fixed at creation, so this cannot be honoured.
///
/// The assertion's point is that nothing reaches the sink: refusing after a
/// write would leave the message in a conversation the person was told had
/// refused it.
#[tokio::test]
async fn a_model_from_the_other_instance_is_refused_before_any_write() {
    let (result, calls) = run(turn(Some(("codex", "gpt-5.6-sol")))).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}

/// Silently substituting the default is the failure the published model names
/// exist to prevent, so an unknown slug refuses rather than falls back.
#[tokio::test]
async fn a_model_we_never_published_is_refused_rather_than_defaulted() {
    let (result, calls) = run(turn(Some(("claudeAgent", "claude-opus-4-8")))).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}

/// The option array the client sends back, keyed by the descriptor ids
/// `providers` published.
fn turn_with_options(instance: &str, slug: &str, options: Value) -> Value {
    let mut command = turn(Some((instance, slug)));
    command["modelSelection"]["options"] = options;
    command
}

fn sent_effort(calls: &[Call]) -> Option<String> {
    match calls.first() {
        Some(Call::SendTurn { effort, .. }) => effort.clone(),
        other => panic!("expected a SendTurn, got {other:?}"),
    }
}

#[tokio::test]
async fn a_selected_effort_reaches_the_sink() {
    let command = turn_with_options(
        "claudeAgent",
        "claude-sonnet-5",
        json!([{ "id": "effort", "value": "xhigh" }]),
    );
    let (_, calls) = run(command).await;
    assert_eq!(sent_effort(&calls).as_deref(), Some("xhigh"));
}

#[tokio::test]
async fn codex_reads_its_effort_off_its_own_descriptor_id() {
    let command = turn_with_options(
        "codex",
        "gpt-5.6-sol",
        json!([{ "id": "reasoningEffort", "value": "minimal" }]),
    );
    let (_, calls) = run_against(command, Some(conversation(AgentProfile::CODEX))).await;
    assert_eq!(sent_effort(&calls).as_deref(), Some("minimal"));
}

/// A selection keyed by an id this server never published for the instance
/// belongs to some other provider; forwarding it would hand an agent a setting
/// it has no vocabulary for.
#[tokio::test]
async fn an_option_the_instance_never_published_is_ignored() {
    let command = turn_with_options(
        "claudeAgent",
        "claude-sonnet-5",
        json!([{ "id": "reasoningEffort", "value": "minimal" }, { "id": "verbosity", "value": "high" }]),
    );
    let (_, calls) = run(command).await;
    assert_eq!(sent_effort(&calls), None);
}

#[tokio::test]
async fn a_turn_that_picked_no_effort_leaves_the_agent_on_its_default() {
    let (_, calls) = run(turn(Some(("claudeAgent", "claude-sonnet-5")))).await;
    assert_eq!(sent_effort(&calls), None);
}

#[tokio::test]
async fn plan_mode_reaches_the_sink_as_the_moatless_plan_mode() {
    let mut command = turn(None);
    command["interactionMode"] = json!("plan");
    let (_, calls) = run(command).await;
    assert!(matches!(
        &calls[0],
        Call::SendTurn { agent_mode, .. } if agent_mode.as_deref() == Some("plan")
    ));
}
