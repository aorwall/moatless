#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for task_event_dao — requires `DATABASE_URL` and
//! `--features db-tests`. Covers the stop fan-out dedupe query that
//! `handle_agent_stopped` guards on.

mod common;

use moatless_vibe_backend::task::dao::{self as task_dao};
use moatless_vibe_backend::task_event::dao::{
    InsertTaskEvent, PgTaskEventRepository, TaskEventRepository,
};
use sqlx::PgPool;

use common::{make_repository_row, make_task_row};

async fn seed_task(pool: &PgPool, task_id: &str) {
    let repo = make_repository_row("repo-1", "user-1");
    moatless_vibe_backend::repository::dao::insert(pool, &repo)
        .await
        .expect("seed repo");
    let task = make_task_row(task_id, "user-1");
    task_dao::insert_task(pool, &task).await.expect("seed task");
}

/// Append an `agent` event, in call order — `id` is a bigserial, so call order
/// is log order, which is exactly what the dedupe compares on.
async fn log_agent_event(repo: &PgTaskEventRepository, task_id: &str, event_id: &str, sub: &str) {
    repo.insert_event(&InsertTaskEvent {
        event_id: event_id.to_string(),
        task_id: task_id.to_string(),
        event_type: "agent".to_string(),
        sub_type: sub.to_string(),
        is_error: sub == "error",
        agent_type: Some("claude-code".to_string()),
        turn_number: Some(1),
        payload: serde_json::json!({}),
        timestamp: "2026-07-12T00:00:00.000Z".to_string(),
    })
    .await
    .expect("log event");
}

#[sqlx::test]
async fn exactly_one_of_a_double_stop_fan_out_survives(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskEventRepository(pool.clone());

    // One user stop, two publishers: the sandbox answering `StopAgent` and the
    // /stop handler. Both rows are in the log before either is dispatched.
    log_agent_event(&repo, "task-a", "started-1", "started").await;
    log_agent_event(&repo, "task-a", "stopped-sandbox", "stopped").await;
    log_agent_event(&repo, "task-a", "stopped-http", "stopped").await;

    assert!(
        !repo
            .agent_run_already_terminal("task-a", "stopped-sandbox")
            .await
            .unwrap(),
        "the first stop logged does the work"
    );
    assert!(
        repo.agent_run_already_terminal("task-a", "stopped-http")
            .await
            .unwrap(),
        "the second stop logged is a duplicate"
    );
}

#[sqlx::test]
async fn a_new_run_reopens_the_guard(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskEventRepository(pool.clone());

    log_agent_event(&repo, "task-a", "started-1", "started").await;
    log_agent_event(&repo, "task-a", "stopped-1", "stopped").await;
    // Next turn: the `started` between the two stops is what keeps the second
    // turn's stop from reading as a duplicate of the first turn's.
    log_agent_event(&repo, "task-a", "started-2", "started").await;
    log_agent_event(&repo, "task-a", "stopped-2", "stopped").await;

    let duplicate = repo
        .agent_run_already_terminal("task-a", "stopped-2")
        .await
        .unwrap();
    assert!(!duplicate);
}

#[sqlx::test]
async fn a_stop_after_the_run_completed_is_dropped(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskEventRepository(pool.clone());

    log_agent_event(&repo, "task-a", "started-1", "started").await;
    log_agent_event(&repo, "task-a", "completed-1", "completed").await;
    log_agent_event(&repo, "task-a", "stopped-late", "stopped").await;

    assert!(
        repo.agent_run_already_terminal("task-a", "stopped-late")
            .await
            .unwrap(),
        "a stop racing the agent's own completion has nothing to add"
    );
}

#[sqlx::test]
async fn an_unlogged_event_fails_open(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskEventRepository(pool.clone());

    log_agent_event(&repo, "task-a", "started-1", "started").await;
    log_agent_event(&repo, "task-a", "stopped-1", "stopped").await;

    // Event persistence is best-effort; a stop whose row never landed must
    // still be handled rather than silently swallowed.
    let duplicate = repo
        .agent_run_already_terminal("task-a", "never-persisted")
        .await
        .unwrap();
    assert!(!duplicate);
}

#[sqlx::test]
async fn other_tasks_and_non_run_state_events_are_ignored(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    seed_task_only(&pool, "task-b").await;
    let repo = PgTaskEventRepository(pool.clone());

    log_agent_event(&repo, "task-b", "stopped-b", "stopped").await;
    log_agent_event(&repo, "task-a", "started-1", "started").await;
    // `message` / `session_init` are not run-state markers.
    log_agent_event(&repo, "task-a", "message-1", "message").await;
    log_agent_event(&repo, "task-a", "stopped-1", "stopped").await;

    let duplicate = repo
        .agent_run_already_terminal("task-a", "stopped-1")
        .await
        .unwrap();
    assert!(!duplicate);
}

/// `seed_task` inserts the shared repository row; a second task reuses it.
async fn seed_task_only(pool: &PgPool, task_id: &str) {
    let task = make_task_row(task_id, "user-1");
    task_dao::insert_task(pool, &task).await.expect("seed task");
}
