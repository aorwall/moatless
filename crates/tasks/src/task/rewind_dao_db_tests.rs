//! PostgreSQL tests for the rewind cut.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::activities::mapping::{
    ActivityKind, ActivityRow, ActivityStatus, BlockContext, ConversationWrite, map_content,
};
use crate::conversation::ids;
use crate::message::models::MessageType;
use crate::message::test_support::{make_message_row, seed_agent_line, seed_message};
use crate::task::models::TaskFork;
use crate::task::rewind_dao::truncate_conversation;
use crate::task::test_support::{make_task_row, seed_task};
use moatless_workspaces::test_support::make_repository_row;

/// A repo and a Task for the Message FKs to land on.
async fn seed_a_task(pool: &PgPool, task_id: &str) {
    let repo = make_repository_row("repo-1", "user-1");
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    seed_task(pool, &make_task_row(task_id, "user-1"))
        .await
        .expect("seed task");
}

/// One Message on `turn`, with one tool call recorded against it — the shape a
/// turn writes. `turn` of `None` is a queued Message no turn has claimed.
async fn seed_turn_message(pool: &PgPool, task_id: &str, uuid: &str, turn: Option<i32>) {
    let mut msg = make_message_row(task_id, uuid);
    msg.message_type = MessageType::Assistant;
    msg.turn_number = turn;
    let conversation = map_content(
        BlockContext {
            task_id,
            message_uuid: uuid,
            parent_uuid: None,
            message_type: MessageType::Assistant,
            block_index: 0,
            source: uuid,
            source_block: 0,
        },
        &serde_json::json!([{
            "type": "tool_use",
            "id": format!("call-{uuid}"),
            "name": "Bash",
            "input": { "command": "cargo build" }
        }]),
    );
    seed_message(pool, &msg, &[], &conversation)
        .await
        .expect("seed message");

    if let Some(turn_number) = turn {
        sqlx::query(
            "INSERT INTO task_turns (task_id, turn_number, status) VALUES ($1, $2, 'completed') \
             ON CONFLICT (task_id, turn_number) DO NOTHING",
        )
        .bind(task_id)
        .bind(turn_number)
        .execute(pool)
        .await
        .expect("seed turn");
    }
}

/// A `sandbox_status` row: it happened on no Message, so no cut can place it.
async fn seed_sandbox_activity(pool: &PgPool, task_id: &str, activity_id: &str) {
    crate::message::test_support::record_sandbox_activity(
        pool,
        &ActivityRow {
            activity_id: activity_id.to_string(),
            task_id: task_id.to_string(),
            ordinal: 0,
            message_uuid: None,
            parent_uuid: None,
            kind: ActivityKind::SandboxStatus,
            status: ActivityStatus::Completed,
            tool_use_id: None,
            summary: "Sandbox running".to_string(),
            payload: serde_json::json!({ "state": "running" }),
            is_error: false,
        },
    )
    .await
    .expect("seed sandbox activity");
}

async fn message_uuids(pool: &PgPool, task_id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT legacy ->> 'uuid' FROM conversation_messages \
          WHERE task_id = $1 AND frozen ORDER BY position",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .expect("read messages")
}

async fn activity_ids(pool: &PgPool, task_id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT activity_id::text FROM conversation_activities \
          WHERE task_id = $1 ORDER BY position, ordinal",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .expect("read activities")
}

async fn turn_numbers(pool: &PgPool, task_id: &str) -> Vec<i32> {
    sqlx::query_scalar("SELECT turn_number FROM task_turns WHERE task_id = $1 ORDER BY turn_number")
        .bind(task_id)
        .fetch_all(pool)
        .await
        .expect("read turns")
}

async fn updated_at(pool: &PgPool, task_id: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT updated_at FROM tasks WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .expect("read task")
}

#[sqlx::test]
async fn the_turns_above_the_cut_go_and_the_rest_stay(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;
    seed_turn_message(&pool, "task-1", "msg-2", Some(2)).await;
    seed_turn_message(&pool, "task-1", "msg-3", Some(3)).await;

    let counts = truncate_conversation(&pool, "task-1", 1, None)
        .await
        .expect("rewind");

    assert_eq!(counts.messages, 2);
    assert_eq!(counts.turns, 2);
    assert_eq!(message_uuids(&pool, "task-1").await, vec!["msg-1"]);
    assert_eq!(turn_numbers(&pool, "task-1").await, vec![1]);
}

#[sqlx::test]
async fn the_cut_turns_are_kept_as_rewound(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;
    seed_turn_message(&pool, "task-1", "msg-2", Some(2)).await;
    seed_turn_message(&pool, "task-1", "msg-3", Some(3)).await;

    truncate_conversation(&pool, "task-1", 1, None)
        .await
        .expect("rewind");

    let rewound: Vec<(i32, String)> = sqlx::query_as(
        "SELECT turn_number, status FROM task_rewound_turns WHERE task_id = $1 ORDER BY turn_number",
    )
    .bind("task-1")
    .fetch_all(&pool)
    .await
    .expect("read rewound turns");
    assert_eq!(
        rewound,
        vec![(2, "completed".to_string()), (3, "completed".to_string())]
    );
}

/// A Message no turn has claimed is pending work rather than history, so the
/// cut leaves it for the next turn to pick up.
#[sqlx::test]
async fn a_queued_message_survives_the_cut(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;
    seed_turn_message(&pool, "task-1", "queued", None).await;

    truncate_conversation(&pool, "task-1", 0, None)
        .await
        .expect("rewind");

    assert_eq!(message_uuids(&pool, "task-1").await, vec!["queued"]);
    assert!(turn_numbers(&pool, "task-1").await.is_empty());
}

/// What happened on a deleted Message goes with it; what happened to the
/// Sandbox stays, because the Sandbox is not rewound.
#[sqlx::test]
async fn a_sandbox_activity_outlives_the_messages_beside_it(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;
    seed_sandbox_activity(&pool, "task-1", "sandbox-1").await;
    seed_turn_message(&pool, "task-1", "msg-2", Some(2)).await;

    truncate_conversation(&pool, "task-1", 1, None)
        .await
        .expect("rewind");

    let left = activity_ids(&pool, "task-1").await;
    assert_eq!(left.len(), 2, "only the deleted Message's call goes");
    assert!(left.contains(&ids::activity("sandbox-1").to_string()));
}

/// The one change that names no row: everything it is about is gone, so a
/// reader applies `reverted_to_turn` instead of re-reading a uuid.
#[sqlx::test]
async fn the_cut_records_a_conversation_change_carrying_the_turn(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;
    seed_turn_message(&pool, "task-1", "msg-2", Some(2)).await;

    truncate_conversation(&pool, "task-1", 1, None)
        .await
        .expect("rewind");

    let (reverted_to_turn, message_uuid): (Option<i32>, Option<String>) = sqlx::query_as(
        "SELECT reverted_to_turn, message_uuid FROM task_conversation_changes \
         WHERE task_id = $1 ORDER BY position DESC LIMIT 1",
    )
    .bind("task-1")
    .fetch_one(&pool)
    .await
    .expect("read the change");
    assert_eq!(reverted_to_turn, Some(1));
    assert_eq!(message_uuid, None);
}

/// `latest_user_message_at`, `pending_question`, `pending_plan` and
/// `assistant_model` are read off the deleted rows, so the thread list has to
/// re-read the row. `updated_at` is what `thread.reverted` is timestamped from.
#[sqlx::test]
async fn the_cut_moves_the_listing_row_and_its_timestamp(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;
    let before = updated_at(&pool, "task-1").await;
    let listing_changes = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM task_listing_changes WHERE task_id = $1")
            .bind("task-1")
            .fetch_one(&pool)
            .await
            .expect("read the listing changes")
    };
    let seeded = listing_changes().await;

    truncate_conversation(&pool, "task-1", 0, None)
        .await
        .expect("rewind");

    assert_eq!(listing_changes().await, seeded + 1);
    assert!(updated_at(&pool, "task-1").await > before);
}

/// The anchor is what makes the next turn branch the agent session at the cut
/// rather than resume the history the person just deleted.
#[sqlx::test]
async fn the_anchor_is_written_with_the_cut(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;
    seed_turn_message(&pool, "task-1", "msg-2", Some(2)).await;

    let cut = seed_agent_line(&pool, "task-1", "session-abc", Some(1), "done").await;
    let anchor = TaskFork {
        task_id: "task-1".to_string(),
        source_task_id: "task-1".to_string(),
        at_turn: Some(1),
        source_event_id: cut,
    };
    truncate_conversation(&pool, "task-1", 1, Some(&anchor))
        .await
        .expect("rewind");

    let row: (String, Option<i32>, uuid::Uuid) = sqlx::query_as(
        "SELECT source_task_id, at_turn, source_event_id FROM task_forks WHERE task_id = $1",
    )
    .bind("task-1")
    .fetch_one(&pool)
    .await
    .expect("read the anchor");
    assert_eq!(row, ("task-1".to_string(), Some(1), cut));
}

/// An emptied Task has no session left to branch, so whatever anchored it goes
/// with the rows: leaving it would have the next turn resume a transcript this
/// Task no longer holds.
#[sqlx::test]
async fn emptying_the_task_clears_the_anchor(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;

    let cut = seed_agent_line(&pool, "task-1", "session-abc", Some(1), "done").await;
    let anchor = TaskFork {
        task_id: "task-1".to_string(),
        source_task_id: "task-1".to_string(),
        at_turn: Some(1),
        source_event_id: cut,
    };
    truncate_conversation(&pool, "task-1", 1, Some(&anchor))
        .await
        .expect("anchor the first cut");
    truncate_conversation(&pool, "task-1", 0, None)
        .await
        .expect("empty the task");

    let anchors: i64 = sqlx::query_scalar("SELECT count(*) FROM task_forks WHERE task_id = $1")
        .bind("task-1")
        .fetch_one(&pool)
        .await
        .expect("read the anchors");
    assert_eq!(anchors, 0);
}

/// Another Task's rows are addressed by `task_id` in every statement of the
/// transaction, so a cut on one conversation cannot reach the other.
#[sqlx::test]
async fn a_cut_reaches_no_other_task(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    seed_task(&pool, &make_task_row("task-2", "user-1"))
        .await
        .expect("seed the second task");
    seed_turn_message(&pool, "task-1", "msg-1", Some(1)).await;
    seed_turn_message(&pool, "task-2", "other-1", Some(1)).await;

    truncate_conversation(&pool, "task-1", 0, None)
        .await
        .expect("rewind");

    assert!(message_uuids(&pool, "task-1").await.is_empty());
    assert_eq!(message_uuids(&pool, "task-2").await, vec!["other-1"]);
    assert_eq!(turn_numbers(&pool, "task-2").await, vec![1]);
}

/// `ConversationWrite`'s default writes nothing, so a Message with no blocks
/// leaves the activity count alone — the counts the log line reports are the
/// rows actually removed.
#[sqlx::test]
async fn a_cut_that_removes_nothing_reports_zeroes(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    let msg = make_message_row("task-1", "msg-1");
    seed_message(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .expect("seed message");

    let counts = truncate_conversation(&pool, "task-1", 4, None)
        .await
        .expect("rewind");

    assert_eq!(counts.messages, 0);
    assert_eq!(counts.turns, 0);
}
