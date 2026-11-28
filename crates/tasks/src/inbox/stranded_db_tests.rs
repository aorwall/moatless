//! Database tests for `dao::stranded`: which waiting items the reaper re-wakes.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::dao;
use crate::message::test_support::{queue_message, user_item};
use crate::task::test_support::seed_bare_task;

/// Seed an open Task holding one pending item sent `minutes_ago`.
async fn seed_waiting(pool: &PgPool, task_id: &str, minutes_ago: i64) {
    seed_bare_task(pool, task_id, "open").await;
    queue_message(pool, user_item(task_id, Uuid::new_v4(), "hello"))
        .await
        .unwrap()
        .expect("queued");
    sqlx::query(
        "UPDATE task_inbox SET created_at = NOW() - make_interval(mins => $2) WHERE task_id = $1",
    )
    .bind(task_id)
    .bind(i32::try_from(minutes_ago).unwrap())
    .execute(pool)
    .await
    .unwrap();
}

async fn set_sandbox(pool: &PgPool, task_id: &str, desired_state: &str, stop_reason: Option<&str>) {
    sqlx::query("UPDATE sandboxes SET desired_state = $2, stop_reason = $3 WHERE sandbox_id = $1")
        .bind(task_id)
        .bind(desired_state)
        .bind(stop_reason)
        .execute(pool)
        .await
        .unwrap();
}

async fn stranded_ids(pool: &PgPool) -> Vec<String> {
    dao::stranded(pool, Utc::now() - Duration::minutes(10))
        .await
        .unwrap()
        .into_iter()
        .map(|(task_id, _)| task_id)
        .collect()
}

#[sqlx::test]
async fn an_old_item_on_a_live_or_reaped_sandbox_is_stranded(pool: PgPool) {
    seed_waiting(&pool, "t-running", 30).await;
    set_sandbox(&pool, "t-running", "running", None).await;
    seed_waiting(&pool, "t-reaped", 20).await;
    set_sandbox(&pool, "t-reaped", "stopped", Some("idle_reaper")).await;

    assert_eq!(stranded_ids(&pool).await, ["t-running", "t-reaped"]);
}

#[sqlx::test]
async fn a_fresh_item_is_not_stranded(pool: PgPool) {
    seed_waiting(&pool, "t1", 2).await;
    set_sandbox(&pool, "t1", "running", None).await;

    assert!(stranded_ids(&pool).await.is_empty());
}

#[sqlx::test]
async fn an_item_an_open_turn_will_take_is_not_stranded(pool: PgPool) {
    seed_waiting(&pool, "t1", 30).await;
    set_sandbox(&pool, "t1", "running", None).await;
    sqlx::query("INSERT INTO task_turns (task_id, turn_number, status) VALUES ('t1', 1, 'active')")
        .execute(&pool)
        .await
        .unwrap();

    assert!(stranded_ids(&pool).await.is_empty());
}

#[sqlx::test]
async fn a_deliberate_stop_or_an_errored_task_is_left_waiting(pool: PgPool) {
    for (task_id, reason) in [
        ("t-user", "user"),
        ("t-crash", "error"),
        ("t-cleanup", "cleanup"),
    ] {
        seed_waiting(&pool, task_id, 30).await;
        set_sandbox(&pool, task_id, "stopped", Some(reason)).await;
    }
    seed_waiting(&pool, "t-removed", 30).await;
    set_sandbox(&pool, "t-removed", "removed", Some("system")).await;
    seed_waiting(&pool, "t-errored", 30).await;
    set_sandbox(&pool, "t-errored", "running", None).await;
    sqlx::query("UPDATE tasks SET status = 'error' WHERE task_id = 't-errored'")
        .execute(&pool)
        .await
        .unwrap();

    assert!(stranded_ids(&pool).await.is_empty());
}
