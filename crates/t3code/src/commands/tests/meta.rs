//! Renaming a thread and changing its model, and when an asserted title is
//! dropped.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn a_meta_update_carries_title_and_translated_model() {
    let command = json!({
        "type": "thread.meta.update",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "title": "Fix the checkout",
        "modelSelection": { "instanceId": "claudeAgent", "model": "claude-haiku-5-5" },
        "branch": "feat/checkout",
        "worktreePath": null
    });
    let (result, calls) = run_against(command, Some(named("Checkout bug"))).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::UpdateMeta {
            task_id: "task-1".to_string(),
            title: Some("Fix the checkout".to_string()),
            model: Some("haiku".to_string()),
        }]
    );
}

/// The client titles a thread from its first message seconds after creating it,
/// before the agent has run. That title is dropped, so the Task reaches the pod
/// unnamed and the agent names it. The model still lands.
#[tokio::test]
async fn a_title_asserted_while_the_thread_is_unnamed_is_dropped() {
    let command = json!({
        "type": "thread.meta.update",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "title": "Doesnt the agent in the sandbox set a task name on...",
        "modelSelection": { "instanceId": "claudeAgent", "model": "claude-haiku-5-5" }
    });
    let (result, calls) = run(command).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::UpdateMeta {
            task_id: "task-1".to_string(),
            title: None,
            model: Some("haiku".to_string()),
        }]
    );
}

/// A name of blanks is no name: the auto-title would otherwise pass through
/// whenever something wrote one.
#[tokio::test]
async fn a_title_asserted_against_a_blank_name_is_dropped() {
    let command = json!({
        "type": "thread.meta.update",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "title": "Fix the checkout"
    });
    let (result, calls) = run_against(command, Some(named("   "))).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::UpdateMeta {
            task_id: "task-1".to_string(),
            title: None,
            model: None,
        }]
    );
}

/// An `instanceId` names a harness, so naming the other one switches to it.
#[tokio::test]
async fn a_meta_update_naming_the_other_instance_switches_its_harness() {
    let command = json!({
        "type": "thread.meta.update",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "title": "Fix the checkout",
        "modelSelection": { "instanceId": "codex", "model": "gpt-5.6-sol" }
    });
    let (result, calls) = run_against(command, Some(named("Checkout bug"))).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![
            Call::Switch {
                task_id: "task-1".to_string(),
                harness: Harness::Codex,
            },
            Call::UpdateMeta {
                task_id: "task-1".to_string(),
                title: Some("Fix the checkout".to_string()),
                model: Some("gpt-5.6-sol".to_string()),
            }
        ]
    );
}

/// The refusal the sink would answer is what the person sees, so an unknown
/// model on a meta update is refused rather than dropped.
#[tokio::test]
async fn a_meta_update_with_an_unpublished_model_is_refused() {
    let command = json!({
        "type": "thread.meta.update",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "modelSelection": { "instanceId": "codex", "model": "gpt-4" }
    });
    let (result, calls) = run_against(command, Some(named("Checkout bug"))).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}

/// Regeneration is a queued turn asking the agent to rename; its `moat
/// set-name` is the answer, so nothing else is recorded.
#[tokio::test]
async fn a_title_regeneration_queues_a_turn() {
    let command = json!({
        "type": "thread.meta.update",
        "commandId": "cmd-regen",
        "threadId": "task-1",
        "regenerateTitle": true
    });
    let (result, calls) = run_against(command, Some(named("Checkout bug"))).await;
    assert!(result.is_ok());
    assert!(
        matches!(
            calls.as_slice(),
            [
                Call::UpdateMeta { title: None, model: None, .. },
                Call::SendTurn { task_id, queue: true, model: None, .. },
            ] if task_id == "task-1"
        ),
        "got {calls:?}"
    );
}
