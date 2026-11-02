//! Changing what a thread has queued, the viewer's read mark, and deleting a
//! thread.

use super::fixtures::*;
use super::*;

fn queue_command(tag: &str, extra: Value) -> Value {
    let mut command = json!({
        "type": tag,
        "commandId": "cmd-1",
        "threadId": "task-1",
        "messageId": "msg-1"
    });
    for (key, value) in extra.as_object().into_iter().flatten() {
        command[key] = value.clone();
    }
    command
}

#[tokio::test]
async fn each_queue_command_reaches_the_sink_as_its_change() {
    for (command, change) in [
        (
            queue_command("thread.queuedTurn.cancel", json!({})),
            "cancel msg-1",
        ),
        (
            queue_command("thread.queuedTurn.edit", json!({ "text": "again" })),
            "edit msg-1 again files=None",
        ),
        (
            queue_command(
                "thread.queuedTurn.edit",
                json!({ "text": "again", "attachments": [] }),
            ),
            "edit msg-1 again files=Some(0)",
        ),
        (
            queue_command(
                "thread.queuedTurn.edit",
                json!({
                    "text": "see [Bug](t3-context://v1/thread/th_1)",
                    "context": { "version": 1, "records": [
                        { "version": 1, "kind": "thread", "contextId": "th_1", "label": "Fix the bug" }
                    ] }
                }),
            ),
            "edit msg-1 see Fix the bug files=None",
        ),
        (
            queue_command(
                "thread.queuedTurn.reorder",
                json!({ "beforeMessageId": "msg-0" }),
            ),
            "reorder msg-1 before=Some(\"msg-0\")",
        ),
        (
            queue_command("thread.queuedTurn.reorder", json!({})),
            "reorder msg-1 before=None",
        ),
        (queue_command("thread.queue.resume", json!({})), "resume"),
    ] {
        let (result, calls) = run(command).await;
        assert!(result.is_ok(), "{change}: {result:?}");
        assert_eq!(
            calls,
            vec![Call::ChangeQueue {
                task_id: "task-1".to_string(),
                change: change.to_string(),
            }]
        );
    }
}

/// The queue is a controller's to change, like the turn it waits for.
#[tokio::test]
async fn a_queue_change_on_an_unreadable_thread_is_refused() {
    let (result, calls) =
        run_against(queue_command("thread.queuedTurn.cancel", json!({})), None).await;
    assert!(
        matches!(result, Err(CommandRefusal::NotWritable { .. })),
        "{result:?}"
    );
    assert!(calls.is_empty());
}

/// A client clock ahead of the server's would otherwise mark activity read
/// before it happened.
#[tokio::test]
async fn a_visit_is_recorded_no_later_than_now() {
    let (result, calls) = run(json!({
        "type": "thread.visit",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "visitedAt": "2026-07-01T09:00:00.000Z"
    }))
    .await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::Visit {
            task_id: "task-1".to_string(),
            visited_at: "2026-07-01T09:00:00+00:00".to_string(),
        }]
    );

    let (result, calls) = run(json!({
        "type": "thread.visit",
        "threadId": "task-1",
        "visitedAt": "2999-01-01T00:00:00.000Z"
    }))
    .await;
    assert!(result.is_ok());
    let [Call::Visit { visited_at, .. }] = calls.as_slice() else {
        panic!("{calls:?}");
    };
    assert!(visited_at.as_str() < "2999", "{visited_at}");
}

#[tokio::test]
async fn mark_unread_and_delete_reach_the_sink() {
    for (tag, expected) in [
        (
            "thread.mark-unread",
            Call::MarkUnread {
                task_id: "task-1".to_string(),
            },
        ),
        (
            "thread.delete",
            Call::Delete {
                only_if_unused: false,
                task_id: "task-1".to_string(),
            },
        ),
    ] {
        let (result, calls) = run(json!({ "type": tag, "threadId": "task-1" })).await;
        assert!(result.is_ok(), "{tag}");
        assert_eq!(calls, vec![expected], "{tag}");
    }
}

#[tokio::test]
async fn closing_a_side_chat_tab_requests_deletion_only_if_unused() {
    let (result, calls) = run(json!({
        "type": "thread.delete", "threadId": "task-1", "onlyIfUnused": true
    }))
    .await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::Delete {
            task_id: "task-1".into(),
            only_if_unused: true
        }]
    );
}
