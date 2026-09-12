//! The commands that reach no sink: the two discarded on purpose, and every
//! one refused because it cannot be honoured.

use super::fixtures::*;
use super::*;

/// Both controls exist on screen and neither has a Moatless meaning.
/// Refusing would put an error behind a control that does nothing.
#[tokio::test]
async fn the_two_mode_commands_are_accepted_and_do_nothing() {
    for command in [
        json!({
            "type": "thread.runtime-mode.set",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "runtimeMode": "full-access",
            "createdAt": "2026-07-01T09:00:00.000Z"
        }),
        json!({
            "type": "thread.interaction-mode.set",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "interactionMode": "plan",
            "createdAt": "2026-07-01T09:00:00.000Z"
        }),
    ] {
        let tag = command["type"].as_str().unwrap().to_string();
        let (result, calls) = run(command).await;
        assert!(result.is_ok(), "{tag}");
        assert!(calls.is_empty(), "{tag}");
    }
}

#[tokio::test]
async fn every_unsupported_command_is_refused_without_a_write() {
    for tag in [
        "thread.delete",
        "project.create",
        "project.delete",
        "thread.approval.respond",
        "thread.checkpoint.revert",
        "thread.something.invented",
    ] {
        let command = json!({
            "type": tag,
            "commandId": "cmd-1",
            "threadId": "task-1"
        });
        let (result, calls) = run(command).await;
        assert!(
            matches!(result, Err(CommandRefusal::Unsupported { .. })),
            "{tag} was not refused"
        );
        assert!(calls.is_empty(), "{tag} reached the sink");
    }
}
