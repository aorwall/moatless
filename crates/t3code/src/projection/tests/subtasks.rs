//! The subtask rows a conversation carries, and the status each reports.

use super::fixtures::*;
use super::*;

/// Field for field against `Subtask` in
/// `packages/contracts/src/subtasks.ts`. Every field there is required, and
/// the three nullable ones are written as null rather than skipped: a
/// client decoding a struct does not get to treat an absent key as null.
#[test]
fn a_subtask_carries_every_field_the_contract_requires() {
    let mut child = running_conversation("task-child", "ws-1");
    child.name = Some("Migrate the callers".to_string());
    child.created_at = "2026-07-01T09:05:00.000Z".to_string();
    child.created_at_db = Some(instant_at("2026-07-01T09:05:00.000Z"));
    child.updated_at = Some(instant_at("2026-07-01T09:11:30.000Z"));

    assert_eq!(
        subtask(&child, TaskRelationType::CreatedBy),
        json!({
            "threadId": "task-child",
            "title": "Migrate the callers",
            "relation": "createdBy",
            "status": "running",
            "awaitingInput": false,
            "agentType": "claude-code",
            "branch": "feat/checkout",
            "turnCount": 3,
            "createdAt": "2026-07-01T09:05:00.000Z",
            "updatedAt": "2026-07-01T09:11:30.000Z",
            "archivedAt": Value::Null,
        })
    );
}

/// A fork is a child too, and the row says which kind it is because the two
/// mean different things to a reader.
#[test]
fn a_forked_child_is_labelled_as_one() {
    assert_eq!(
        subtask(
            &conversation("task-fork", "ws-1"),
            TaskRelationType::ForkedFrom
        )["relation"],
        "forkedFrom"
    );
}

/// Closing outranks the newest turn. A child whose last turn errored and
/// which was then closed is finished, and reporting it as failing would
/// keep a settled fleet looking broken.
#[test]
fn subtask_status_reads_closing_before_the_newest_turn() {
    let mut child = running_conversation("task-child", "ws-1");
    child.turn_status = Some("error".to_string());
    assert_eq!(
        subtask(&child, TaskRelationType::CreatedBy)["status"],
        "failed"
    );

    child.closed_at = Some(Utc::now());
    assert_eq!(
        subtask(&child, TaskRelationType::CreatedBy)["status"],
        "completed"
    );
}

/// The four turn states a child can settle in, and the one it sits at
/// before it has ever run.
#[test]
fn every_turn_state_maps_onto_the_panels_vocabulary() {
    let mut child = running_conversation("task-child", "ws-1");
    for (turn_status, expected) in [
        ("active", "running"),
        // The Sandbox went away mid-turn; the run resumes, so it is working.
        ("interrupted", "running"),
        ("completed", "idle"),
        ("error", "failed"),
        ("stopped", "interrupted"),
        // A status this build does not know is not reported as a state it
        // is not in.
        ("something-newer", "pending"),
    ] {
        child.turn_status = Some(turn_status.to_string());
        assert_eq!(
            subtask(&child, TaskRelationType::CreatedBy)["status"],
            expected,
            "turn status {turn_status}"
        );
    }

    child.turn_status = None;
    assert_eq!(
        subtask(&child, TaskRelationType::CreatedBy)["status"],
        "pending"
    );
}

/// Waiting on a person is not a state of the run, so it rides beside the
/// status rather than replacing it.
#[test]
fn awaiting_input_is_reported_beside_a_running_status() {
    let mut child = running_conversation("task-child", "ws-1");
    child.pending_question = true;
    let row = subtask(&child, TaskRelationType::CreatedBy);
    assert_eq!(row["awaitingInput"], true);
    assert_eq!(row["status"], "running");

    child.pending_question = false;
    child.pending_plan = true;
    assert_eq!(
        subtask(&child, TaskRelationType::CreatedBy)["awaitingInput"],
        true
    );
}

/// An unnamed child gets the contract's placeholder: `title` refuses a
/// blank string, and `tasks.name` is nullable.
#[test]
fn an_unnamed_subtask_falls_back_to_the_contracts_placeholder() {
    let mut child = conversation("task-child", "ws-1");
    child.name = Some("   ".to_string());
    child.branch = None;
    child.turn_number = None;
    let row = subtask(&child, TaskRelationType::CreatedBy);
    assert_eq!(row["title"], "New thread");
    assert_eq!(row["branch"], Value::Null);
    assert_eq!(row["turnCount"], 0);
}
