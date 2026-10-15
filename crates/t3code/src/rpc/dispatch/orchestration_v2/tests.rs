use super::*;

fn launch(payload: Value) -> OrchestrationV2ThreadLaunchInput {
    serde_json::from_value(payload).expect("a launch input")
}

#[test]
fn a_launch_with_a_message_bootstraps_the_thread_and_starts_its_turn() {
    let command = launch_command(
        &launch(json!({
            "projectId": "ws-a",
            "modelSelection": { "model": "opus" },
            "workspaceStrategy": { "type": "worktree", "branch": "feat/x" },
            "initialMessage": { "messageId": "m1", "text": "hello" }
        })),
        "task-1",
    )
    .expect("a command");
    assert_eq!(command["type"], json!("thread.turn.start"));
    assert_eq!(command["threadId"], json!("task-1"));
    assert_eq!(command["message"]["messageId"], json!("m1"));
    assert_eq!(command["message"]["attachments"], json!([]));
    assert_eq!(
        command["bootstrap"]["createThread"]["projectId"],
        json!("ws-a")
    );
    assert_eq!(
        command["bootstrap"]["createThread"]["branch"],
        json!("feat/x")
    );
}

#[test]
fn a_launch_into_an_existing_thread_carries_no_bootstrap() {
    let command = launch_command(
        &launch(json!({
            "projectId": "ws-a",
            "reuseExistingThread": true,
            "initialMessage": { "messageId": "m1", "text": "hello" }
        })),
        "task-1",
    )
    .expect("a command");
    assert!(command.get("bootstrap").is_none());
}

#[test]
fn a_launch_without_a_message_only_creates_the_thread() {
    let command =
        launch_command(&launch(json!({ "projectId": "ws-a" })), "task-1").expect("a command");
    assert_eq!(command["type"], json!("thread.create"));
    assert_eq!(command["projectId"], json!("ws-a"));
}

#[test]
fn reusing_a_thread_with_nothing_to_send_issues_no_command() {
    let payload = launch(json!({ "projectId": "ws-a", "reuseExistingThread": true }));
    assert_eq!(launch_command(&payload, "task-1"), None);
}
