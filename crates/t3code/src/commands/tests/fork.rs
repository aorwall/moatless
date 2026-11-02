//! Forking a thread: where the cut lands, and what the fork carries over.

use super::fixtures::*;
use super::*;

/// The command a hover-icon fork sends with nothing else picked: the source
/// thread and where to cut, nothing more.
fn fork_command() -> Value {
    json!({
        "type": "thread.fork",
        "commandId": "cmd-1",
        "threadId": "fork-1",
        "sourceThreadId": "task-1",
        "createdAt": "2026-07-01T09:00:00.000Z"
    })
}

#[tokio::test]
async fn a_fork_persists_under_the_id_the_client_minted() {
    let (result, calls) = run(fork_command()).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::Fork {
            side_chat: false,
            title: None,
            new_task_id: "fork-1".to_string(),
            source_task_id: "task-1".to_string(),
            at_turn: None,
            message: None,
            same_sandbox: false,
            model: None,
            agent_mode: None,
            effort: None,
        }]
    );
}

/// `atTurn`, `sameSandbox` and `message` reach the sink verbatim — the same
/// shape the HTTP fork request carries. A `branch` in the command is dropped: a
/// fork runs on the branch its source is already on.
#[tokio::test]
async fn a_fork_carries_at_turn_same_sandbox_and_message_but_not_branch() {
    let mut command = fork_command();
    command["atTurn"] = json!(3);
    command["sameSandbox"] = json!(false);
    command["branch"] = json!("feat/try-again");
    command["message"] = json!("try a different approach");
    let (result, calls) = run(command).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::Fork {
            side_chat: false,
            title: None,
            new_task_id: "fork-1".to_string(),
            source_task_id: "task-1".to_string(),
            at_turn: Some(3),
            message: Some("try a different approach".to_string()),
            same_sandbox: false,
            model: None,
            agent_mode: None,
            effort: None,
        }]
    );
}

/// A model override is checked and translated against the *source* thread's
/// agent, the same way `meta_update` checks one against the thread it is
/// updating — the fork has no agent of its own yet.
#[tokio::test]
async fn a_fork_carries_a_translated_model_override() {
    let mut command = fork_command();
    command["modelSelection"] = json!({ "instanceId": "claudeAgent", "model": "claude-haiku-5-5" });
    let (result, calls) = run(command).await;
    assert!(result.is_ok());
    assert!(matches!(
        calls.as_slice(),
        [Call::Fork { model, .. }] if model.as_deref() == Some("haiku")
    ));
}

#[tokio::test]
async fn a_fork_without_a_source_thread_id_is_refused() {
    let mut command = fork_command();
    command.as_object_mut().unwrap().remove("sourceThreadId");
    let (result, calls) = run(command).await;
    assert!(calls.is_empty());
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
}

#[tokio::test]
async fn a_fork_at_turn_that_is_not_a_whole_number_is_refused() {
    let mut command = fork_command();
    command["atTurn"] = json!("three");
    let (result, calls) = run(command).await;
    assert!(calls.is_empty());
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
}

/// A source thread the viewer cannot read is reported the same as one that does
/// not exist — existence hiding, as everywhere else on this surface.
#[tokio::test]
async fn a_fork_of_a_thread_the_viewer_cannot_read_is_refused() {
    let (result, calls) = run_against(fork_command(), None).await;
    assert!(calls.is_empty());
    assert!(matches!(
        result,
        Err(CommandRefusal::NotWritable { task_id }) if task_id == "task-1"
    ));
}

/// An `instanceId` naming another harness than the source's runs the fork on
/// it, with the model translated for that instance.
#[tokio::test]
async fn a_fork_naming_the_other_instance_runs_on_its_harness() {
    let mut command = fork_command();
    command["modelSelection"] = json!({ "instanceId": "codex", "model": "gpt-5.6-sol" });
    let (result, calls) = run(command).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![
            Call::Switch {
                task_id: "fork-1".to_string(),
                harness: Harness::Codex,
            },
            Call::Fork {
                side_chat: false,
                title: None,
                new_task_id: "fork-1".to_string(),
                source_task_id: "task-1".to_string(),
                at_turn: None,
                message: None,
                same_sandbox: false,
                model: Some("gpt-5.6-sol".to_string()),
                agent_mode: None,
                effort: None,
            }
        ]
    );
}

#[tokio::test]
async fn side_chat_title_and_interaction_mode_reach_creation() {
    let mut command = fork_command();
    command["sideChat"] = json!(true);
    command["title"] = json!("Side chat");
    command["interactionMode"] = json!("plan");
    let (result, calls) = run(command).await;
    assert!(result.is_ok());
    assert!(
        matches!(calls.as_slice(), [Call::Fork { side_chat: true, title, agent_mode, .. }]
        if title.as_deref() == Some("Side chat") && agent_mode.as_deref() == Some("plan"))
    );
}
