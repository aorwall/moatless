//! Which turns become a checkpoint, and what each one names.

use super::contract::*;
use super::fixtures::*;
use super::*;

// ── Checkpoints ─────────────────────────────────────────────────────

/// The thread's start, then one per settled turn, each captured when its
/// closing bubble was written and chained to the one before.
#[test]
fn a_checkpoint_per_turn_captured_at_its_closing_bubble() {
    let mut messages = vec![
        message(1, MessageType::User, "do a thing"),
        message(2, MessageType::Assistant, "did it"),
        message(3, MessageType::User, "and another"),
        message(4, MessageType::Assistant, "did that too"),
    ];
    messages[0].turn_number = Some(1);
    messages[1].turn_number = Some(1);

    let detail = quiet_detail(&transcript(messages, Vec::new()));
    let checkpoints = detail["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 3);
    for checkpoint in checkpoints {
        assert_keys(
            checkpoint,
            "checkpoint",
            CHECKPOINT_REQUIRED,
            CHECKPOINT_OPTIONAL,
        );
    }
    assert_eq!(checkpoints[0]["runId"], Value::Null);
    assert_eq!(checkpoints[0]["ordinalWithinScope"], json!(0));
    assert_eq!(checkpoints[1]["runId"], "task-a:1");
    assert_eq!(checkpoints[1]["ref"], "task-a:checkpoint:1");
    assert_eq!(checkpoints[1]["appRunOrdinal"], json!(1));
    assert_eq!(checkpoints[1]["capturedAt"], "2026-07-01T10:02:00.000Z");
    assert_eq!(checkpoints[1]["parentCheckpointId"], checkpoints[0]["id"]);
    assert_eq!(checkpoints[2]["runId"], "task-a:3");
    assert_eq!(checkpoints[2]["appRunOrdinal"], json!(3));
    assert_eq!(checkpoints[2]["parentCheckpointId"], checkpoints[1]["id"]);
}

/// A turn still running has not completed, so it is no cut. The ones that
/// ended below it are, and the client greys their control out while the thread
/// works rather than hiding where a rewind would land.
#[test]
fn a_running_turn_withholds_itself_and_nothing_below_it() {
    let mut done = message(1, MessageType::Assistant, "did it");
    done.turn_number = Some(1);
    let running = message(2, MessageType::Assistant, "doing it");
    let detail = detail(&transcript(vec![done, running], Vec::new()));
    let checkpoints = detail["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[1]["appRunOrdinal"], json!(1));
}

/// `ready` with no files, never `missing`: the client defers on `missing` and
/// would re-examine the turn for a diff for as long as the thread is open, and
/// this backend has no worktree snapshot to hand it either way.
#[test]
fn a_checkpoint_is_ready_and_carries_no_files() {
    let mut message = message(1, MessageType::Assistant, "did it");
    message.turn_number = Some(1);
    let detail = quiet_detail(&transcript(vec![message], Vec::new()));
    for checkpoint in detail["checkpoints"].as_array().unwrap() {
        assert_eq!(checkpoint["status"], "ready");
        assert_eq!(checkpoint["files"], json!([]));
    }
}

/// A queued Message has no Turn, and a checkpoint the client cannot revert to
/// would be a control that refuses when pressed.
#[test]
fn a_queued_message_is_no_checkpoint() {
    let mut message = message(1, MessageType::User, "not picked up yet");
    message.turn_number = None;
    let detail = quiet_detail(&transcript(vec![message], Vec::new()));
    let checkpoints = detail["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 2);
    for checkpoint in checkpoints {
        let run = checkpoint["runId"].as_str().unwrap_or_default();
        assert!(
            !run.contains(":queued:"),
            "a queued run must not be a checkpoint"
        );
    }
}

#[test]
fn an_empty_conversation_still_decodes() {
    let detail = detail(&transcript(Vec::new(), Vec::new()));
    assert_keys(&detail, "projection", PROJECTION_REQUIRED, &[]);
    assert_eq!(detail["messages"], json!([]));
    assert_eq!(detail["turnItems"], json!([]));
    assert_eq!(detail["plans"], json!([]));
    assert_eq!(detail["thread"]["deletedAt"], Value::Null);
    let checkpoints = detail["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 1, "only the thread's start");
    assert_eq!(checkpoints[0]["runId"], Value::Null);
}

/// An errored newest turn is `error`, so the client tells a failed cut from a
/// completed one.
#[test]
fn an_errored_newest_turn_is_an_error_checkpoint() {
    let mut earlier = message(1, MessageType::Assistant, "did it");
    earlier.turn_number = Some(1);
    let failed = message(2, MessageType::Assistant, "gave up");
    let mut conversation = running_conversation("task-a", "ws-a");
    conversation.turn_status = Some("error".to_string());
    let detail = detail_of(conversation, &transcript(vec![earlier, failed], Vec::new()));
    assert_eq!(detail["checkpoints"][1]["status"], "ready");
    assert_eq!(detail["checkpoints"][2]["status"], "error");
}
