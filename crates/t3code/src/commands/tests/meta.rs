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
        "modelSelection": { "instanceId": "claudeAgent", "model": "claude-haiku-4-5" },
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
        "modelSelection": { "instanceId": "claudeAgent", "model": "claude-haiku-4-5" }
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

/// The client sends a meta update before every turn. Refusing one because it
/// names the other instance's model would abort the send before the turn itself
/// could explain why.
#[tokio::test]
async fn a_meta_update_naming_the_other_instance_still_updates_the_title() {
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
        vec![Call::UpdateMeta {
            task_id: "task-1".to_string(),
            title: Some("Fix the checkout".to_string()),
            model: None,
        }]
    );
}
