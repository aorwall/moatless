//! Which turns become a checkpoint, and what each one names.

use super::contract::*;
use super::fixtures::*;
use super::*;

// ── Checkpoints ─────────────────────────────────────────────────────

#[test]
fn a_checkpoint_per_turn_labelled_by_its_closing_bubble() {
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
    assert_eq!(checkpoints.len(), 2);
    assert_keys(
        &checkpoints[0],
        "checkpoint",
        CHECKPOINT_REQUIRED,
        CHECKPOINT_OPTIONAL,
    );
    assert_eq!(checkpoints[0]["turnId"], "task-a:1");
    assert_eq!(checkpoints[0]["checkpointRef"], "task-a:1");
    assert_eq!(checkpoints[0]["checkpointTurnCount"], json!(1));
    assert_eq!(checkpoints[0]["assistantMessageId"], "msg-2");
    assert_eq!(checkpoints[0]["completedAt"], "2026-07-01T10:02:00.000Z");
    assert_eq!(checkpoints[1]["checkpointTurnCount"], json!(3));
    assert_eq!(checkpoints[1]["assistantMessageId"], "msg-4");
}

/// `rewind` refuses every cut while a turn is open — including one below it,
/// unlike a fork — so offering any checkpoint would be offering a control
/// that refuses when pressed.
#[test]
fn no_checkpoint_while_a_turn_is_in_flight() {
    let mut message = message(1, MessageType::Assistant, "did it");
    message.turn_number = Some(1);
    let detail = detail(&transcript(vec![message], Vec::new()));
    assert_eq!(detail["checkpoints"], json!([]));
}

/// `ready` with no files, never `missing`: the client defers on `missing`
/// and would re-examine the turn for a diff for as long as the thread is
/// open, and this backend has no worktree snapshot to hand it either way.
#[test]
fn a_checkpoint_is_ready_and_carries_no_files() {
    let mut message = message(1, MessageType::Assistant, "did it");
    message.turn_number = Some(1);
    let detail = quiet_detail(&transcript(vec![message], Vec::new()));
    assert_eq!(detail["checkpoints"][0]["status"], "ready");
    assert_eq!(detail["checkpoints"][0]["files"], json!([]));
}

/// A turn whose agent only called tools wrote no bubble to name.
#[test]
fn a_checkpoint_on_a_silent_turn_names_no_assistant_message() {
    let mut message = message(1, MessageType::User, "do a thing");
    message.turn_number = Some(1);
    let detail = quiet_detail(&transcript(vec![message], Vec::new()));
    assert_eq!(detail["checkpoints"][0]["assistantMessageId"], Value::Null);
}

/// The turns are `task_turns`, not the Messages: a turn past
/// `TRANSCRIPT_LIMIT` is one a rewind still takes, and withholding it would
/// make a long thread's older turns silently unrewindable.
#[test]
fn a_turn_with_no_message_in_the_window_is_still_a_checkpoint() {
    let mut rows = transcript(Vec::new(), Vec::new());
    rows.turns = vec![7];
    let mut conversation = running_conversation("task-a", "ws-a");
    conversation.turn_status = Some("completed".to_string());
    conversation.created_at_db = Some(instant_at("2026-07-01T09:00:00Z"));
    let detail = detail_of(conversation, &rows);
    let checkpoints = detail["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0]["checkpointTurnCount"], json!(7));
    assert_eq!(checkpoints[0]["assistantMessageId"], Value::Null);
    assert_eq!(checkpoints[0]["completedAt"], "2026-07-01T09:00:00.000Z");
}

/// A queued Message has no Turn, and a checkpoint the client cannot revert
/// to would be a control that refuses when pressed.
#[test]
fn a_queued_message_is_no_checkpoint() {
    let mut message = message(1, MessageType::User, "not picked up yet");
    message.turn_number = None;
    let detail = quiet_detail(&transcript(vec![message], Vec::new()));
    assert_eq!(detail["checkpoints"], json!([]));
}

#[test]
fn an_empty_conversation_still_decodes() {
    let detail = detail(&transcript(Vec::new(), Vec::new()));
    assert_keys(&detail, "thread", DETAIL_REQUIRED, DETAIL_OPTIONAL);
    assert_eq!(detail["messages"], json!([]));
    assert_eq!(detail["activities"], json!([]));
    assert_eq!(detail["checkpoints"], json!([]));
    assert_eq!(detail["deletedAt"], Value::Null);
}
