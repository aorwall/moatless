//! Creating a thread, bootstrapped with a first message or standalone.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn a_bootstrapped_turn_creates_the_thread_with_its_first_message() {
    let mut command = turn(Some(("claudeAgent", "claude-opus-5")));
    command["bootstrap"] = json!({
        "createThread": {
            "projectId": "ws-1",
            "title": "New chat",
            "runtimeMode": "full-access",
            "interactionMode": "default",
            "branch": null,
            "worktreePath": null,
            "createdAt": "2026-07-01T09:00:00.000Z"
        }
    });
    // A bootstrapped turn never resolves an existing thread — there is not
    // one yet.
    let (result, calls) = run_against(command, None).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::CreateThread {
            task_id: "task-1".to_string(),
            workspace_id: "ws-1".to_string(),
            instance: "claudeAgent".to_string(),
            branch: None,
            has_first_message: true,
            provision: false,
            skills: vec![],
        }]
    );
}

/// The branch the person picked in the composer reaches the sink, so the Task
/// is created against it rather than the repository's default.
#[tokio::test]
async fn a_bootstrapped_turn_carries_the_picked_branch() {
    let mut command = turn(Some(("claudeAgent", "claude-opus-5")));
    command["bootstrap"] = json!({
        "createThread": {
            "projectId": "ws-1",
            "title": "New chat",
            "runtimeMode": "full-access",
            "interactionMode": "default",
            "branch": "feat/checkout",
            "worktreePath": null,
            "createdAt": "2026-07-01T09:00:00.000Z"
        }
    });
    let (result, calls) = run_against(command, None).await;
    assert!(result.is_ok());
    assert!(matches!(
        calls.as_slice(),
        [Call::CreateThread { branch, .. }] if branch.as_deref() == Some("feat/checkout")
    ));
}

#[tokio::test]
async fn a_standalone_create_carries_no_first_message() {
    let command = json!({
        "type": "thread.create",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "projectId": "ws-1",
        "title": "New chat",
        "modelSelection": { "instanceId": "codex", "model": "gpt-5.6-sol" },
        "runtimeMode": "full-access",
        "interactionMode": "default",
        "branch": null,
        "worktreePath": null,
        "createdAt": "2026-07-01T09:00:00.000Z"
    });
    let (result, calls) = run_against(command, None).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::CreateThread {
            task_id: "task-1".to_string(),
            workspace_id: "ws-1".to_string(),
            instance: "codex".to_string(),
            branch: None,
            has_first_message: false,
            provision: false,
            skills: vec![],
        }]
    );
}

/// The draft's Script run: the Task is created for the Sandbox rather than for
/// a turn, so the create is the only thing that will ever ask for one.
#[tokio::test]
async fn a_standalone_create_can_ask_for_a_sandbox() {
    let mut command = json!({
        "type": "thread.create",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "projectId": "ws-1",
        "title": "Dev server",
        "modelSelection": { "instanceId": "codex", "model": "gpt-5.6-sol" },
        "runtimeMode": "full-access",
        "interactionMode": "default",
        "branch": null,
        "worktreePath": null,
        "createdAt": "2026-07-01T09:00:00.000Z"
    });
    command["provision"] = json!(true);
    let (result, calls) = run_against(command, None).await;
    assert!(result.is_ok());
    assert!(matches!(
        calls.as_slice(),
        [Call::CreateThread {
            provision: true,
            ..
        }]
    ));
}
