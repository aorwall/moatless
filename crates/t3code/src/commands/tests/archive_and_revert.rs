//! Archiving a thread and cutting its transcript — the two commands whose
//! Moatless meaning differs from the T3 name.

use super::fixtures::*;
use super::*;

/// Archiving is closing and unarchiving is reopening, so both reach the sink
/// rather than a refusal. This is the pair a person uses to put a finished
/// conversation away and to pull it back out.
#[tokio::test]
async fn archive_and_unarchive_reach_the_sink() {
    for (tag, expected) in [
        (
            "thread.archive",
            Call::Archive {
                task_id: "task-1".to_string(),
            },
        ),
        (
            "thread.unarchive",
            Call::Unarchive {
                task_id: "task-1".to_string(),
            },
        ),
    ] {
        let (result, calls) = run(json!({
            "type": tag,
            "commandId": "cmd-1",
            "threadId": "task-1",
            "createdAt": "2026-07-01T09:00:00.000Z"
        }))
        .await;
        assert!(result.is_ok(), "{tag}: {result:?}");
        assert_eq!(calls, vec![expected], "{tag}");
    }
}

/// A command with no thread on it is rejected before anything is written.
#[tokio::test]
async fn archiving_without_a_thread_id_writes_nothing() {
    let (result, calls) = run(json!({ "type": "thread.archive", "commandId": "cmd-1" })).await;
    assert!(
        matches!(result, Err(CommandRefusal::Rejected { .. })),
        "{result:?}"
    );
    assert!(calls.is_empty());
}

/// `turnCount` is the cut itself and reaches the sink unchanged: it is the
/// `checkpointTurnCount` this server reported, which is a `task_turns` number,
/// so a translation on either side would move the cut.
#[tokio::test]
async fn a_conversation_revert_carries_its_turn_count_through() {
    let (result, calls) = run(json!({
        "type": "thread.conversation.revert",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "turnCount": 4
    }))
    .await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        calls,
        vec![Call::Revert {
            task_id: "task-1".to_string(),
            keep_through: 4
        }]
    );
}

/// `0` empties the thread, and is a cut like any other.
#[tokio::test]
async fn a_conversation_revert_to_zero_reaches_the_sink() {
    let (result, calls) = run(json!({
        "type": "thread.conversation.revert",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "turnCount": 0
    }))
    .await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        calls,
        vec![Call::Revert {
            task_id: "task-1".to_string(),
            keep_through: 0
        }]
    );
}

#[tokio::test]
async fn a_conversation_revert_without_a_usable_turn_count_writes_nothing() {
    for turn_count in [json!(-1), json!("4"), Value::Null] {
        let (result, calls) = run(json!({
            "type": "thread.conversation.revert",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "turnCount": turn_count
        }))
        .await;
        assert!(
            matches!(result, Err(CommandRefusal::Rejected { .. })),
            "{turn_count} was not rejected: {result:?}"
        );
        assert!(calls.is_empty(), "{turn_count} reached the sink");
    }
}
