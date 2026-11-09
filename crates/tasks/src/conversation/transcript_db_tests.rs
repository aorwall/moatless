//! PostgreSQL tests for where a settled call's output is stored and how a
//! transcript read puts it back.

use serde_json::{Value, json};
use sqlx::PgPool;

use super::transcript::TranscriptReads;
use crate::activities::dao as activities;
use crate::task::test_support::make_task_row;
use crate::trajectory::dao::PgTrajectoryDao;
use crate::trajectory::models::{AppendRequest, NewEvent};
use moatless_types::agent::Harness;
use moatless_workspaces::test_support::make_repository_row;

const TASK: &str = "task-1";

/// A Task with one Bash call, settled with `output`.
async fn settled_call(pool: &PgPool, output: &str) {
    let repo = make_repository_row("repo-1", "user-1");
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .unwrap();
    crate::task::test_support::seed_task(pool, &make_task_row(TASK, "user-1"))
        .await
        .unwrap();
    let said = |item: &str, role: &str, blocks: Value| NewEvent {
        kind: "line".into(),
        harness_item_id: Some(item.into()),
        harness_turn_id: None,
        payload: "{}".into(),
        content: json!({ "role": role, "blocks": blocks }),
    };
    let lines = [
        said(
            "msg_1",
            "assistant",
            json!([{ "type": "tool_use", "id": "toolu_1", "name": "Bash",
                     "input": { "command": "ls" } }]),
        ),
        said(
            "u_1",
            "user",
            json!([{ "type": "tool_result", "tool_use_id": "toolu_1",
                     "content": output, "is_error": false }]),
        ),
    ];
    PgTrajectoryDao(pool.clone())
        .append(AppendRequest {
            task_id: TASK,
            harness: Harness::ClaudeCode,
            harness_session_id: "s",
            first_seq: 0,
            lines: &lines,
            turn_number: Some(1),
            subagent_parent: None,
            own_from_seq: None,
        })
        .await
        .unwrap();
}

/// `(result column, payload's own result)` of the call.
async fn stored(pool: &PgPool) -> (Option<Value>, Option<Value>) {
    sqlx::query_as(
        "SELECT result, payload -> 'result' FROM conversation_activities WHERE task_id = $1",
    )
    .bind(TASK)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The call's output as each read returns it: the transcript and the lookup.
async fn read_back(pool: &PgPool, reads: &TranscriptReads) -> (Value, Value) {
    let window = reads.read(TASK).await.unwrap();
    let [call] = window.activities.as_slice() else {
        panic!("one call, got {:?}", window.activities);
    };
    let found = activities::find_tool_call(pool, TASK, "toolu_1")
        .await
        .unwrap()
        .unwrap();
    (
        call.payload["result"].clone(),
        found.payload["result"].clone(),
    )
}

#[sqlx::test]
async fn a_settled_calls_output_is_stored_beside_its_payload(pool: PgPool) {
    settled_call(&pool, "a.rs").await;

    let (column, in_payload) = stored(&pool).await;
    assert_eq!(column, Some(json!("a.rs")));
    assert_eq!(in_payload, None);
    let reads = TranscriptReads::new(pool.clone());
    assert_eq!(
        read_back(&pool, &reads).await,
        (json!("a.rs"), json!("a.rs"))
    );
}

/// A row settled before the `result` column holds its output in `payload`.
#[sqlx::test]
async fn a_call_settled_before_the_column_keeps_its_output(pool: PgPool) {
    settled_call(&pool, "a.rs").await;
    sqlx::query(
        "UPDATE conversation_activities
            SET payload = payload || jsonb_build_object('result', result), result = NULL
          WHERE task_id = $1",
    )
    .bind(TASK)
    .execute(&pool)
    .await
    .unwrap();

    let reads = TranscriptReads::new(pool.clone());
    assert_eq!(
        read_back(&pool, &reads).await,
        (json!("a.rs"), json!("a.rs"))
    );
}

#[sqlx::test]
async fn a_rewritten_output_is_read_again_rather_than_served_from_the_cache(pool: PgPool) {
    settled_call(&pool, "a.rs").await;
    let reads = TranscriptReads::new(pool.clone());
    read_back(&pool, &reads).await;

    sqlx::query("UPDATE conversation_activities SET result = '\"b.rs\"' WHERE task_id = $1")
        .bind(TASK)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        read_back(&pool, &reads).await,
        (json!("b.rs"), json!("b.rs"))
    );
}
