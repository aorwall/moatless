#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for "Closed is terminal against automated re-open" (Fix A).
//!
//! Covers:
//! - `TaskRepository::update_status_if_not_closed` — race-safe conditional flip.
//! - The three agent terminal handlers (`completed` / `stopped` / `error`) no
//!   longer resurrect a `Closed` task to `Open`.
//! - Regression: a synthetic `agent.error` (as the orphaned-turn reaper emits)
//!   keeps a closed task closed while still appending its error message row.

mod common;

use moatless_vibe_backend::event_handler::agent_handlers::{
    handle_agent_completed, handle_agent_error, handle_agent_stopped,
};
use moatless_vibe_backend::event_handler::models::AgentEvent;
use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::task::dao::{
    self as task_dao, PgTaskRepository, TaskPatch, TaskRepository,
};
use moatless_vibe_backend::task::models::TaskStatus;
use sqlx::PgPool;

use common::{make_repository_row, make_task_row, test_app_state};

/// Seed a repo + task with the given status and return an `AppState`.
async fn seed_with_status(pool: &PgPool, task_id: &str, status: TaskStatus) {
    let repo = make_repository_row("repo-1", "user-1");
    repository_dao::insert(pool, &repo).await.unwrap();
    let task = make_task_row(task_id, "user-1");
    task_dao::insert_task(pool, &task).await.unwrap();
    if status != TaskStatus::Open {
        PgTaskRepository(pool.clone())
            .update_task(
                task_id,
                &TaskPatch {
                    status: Some(status),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
}

async fn status_of(pool: &PgPool, task_id: &str) -> TaskStatus {
    task_dao::find_by_id_internal(pool, task_id)
        .await
        .unwrap()
        .expect("task present")
        .status
}

fn terminal_event(
    event_id: &str,
    task_id: &str,
    sub_type: &str,
    payload: serde_json::Value,
) -> AgentEvent {
    let value = serde_json::json!({
        "eventId": event_id,
        "type": "agent",
        "subType": sub_type,
        "taskId": task_id,
        "timestamp": "2026-04-30T00:00:00.000Z",
        "isError": sub_type == "error",
        "agentType": "claude-code",
        "turnNumber": 1,
        "payload": payload,
    });
    serde_json::from_value(value).unwrap()
}

// =========================================================================
// update_status_if_not_closed (DAO)
// =========================================================================

#[sqlx::test]
async fn update_status_if_not_closed_noop_on_closed(pool: PgPool) {
    seed_with_status(&pool, "task-closed", TaskStatus::Closed).await;
    let repo = PgTaskRepository(pool.clone());

    let changed = repo
        .update_status_if_not_closed("task-closed", TaskStatus::Open)
        .await
        .unwrap();

    assert!(!changed, "a closed task must not be flipped");
    assert_eq!(status_of(&pool, "task-closed").await, TaskStatus::Closed);
}

#[sqlx::test]
async fn update_status_if_not_closed_flips_open_and_error(pool: PgPool) {
    let repo = PgTaskRepository(pool.clone());

    // Open → Open still matches the predicate (status <> 'closed').
    seed_with_status(&pool, "task-open", TaskStatus::Open).await;
    assert!(repo
        .update_status_if_not_closed("task-open", TaskStatus::Open)
        .await
        .unwrap());
    assert_eq!(status_of(&pool, "task-open").await, TaskStatus::Open);

    // Error → Open flips (preserves the start_task-from-Error path).
    PgTaskRepository(pool.clone())
        .update_task(
            "task-open",
            &TaskPatch {
                status: Some(TaskStatus::Error),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(repo
        .update_status_if_not_closed("task-open", TaskStatus::Open)
        .await
        .unwrap());
    assert_eq!(status_of(&pool, "task-open").await, TaskStatus::Open);
}

// =========================================================================
// Terminal handlers keep a Closed task closed
// =========================================================================

#[sqlx::test]
async fn completed_does_not_reopen_closed_task(pool: PgPool) {
    seed_with_status(&pool, "task-c", TaskStatus::Closed).await;
    let state = test_app_state(pool.clone());

    let event = terminal_event(
        "evt-c",
        "task-c",
        "completed",
        serde_json::json!({ "is_error": false, "total_cost_usd": 0.01 }),
    );
    handle_agent_completed(&state, &event).await;

    assert_eq!(status_of(&pool, "task-c").await, TaskStatus::Closed);
}

#[sqlx::test]
async fn stopped_does_not_reopen_closed_task(pool: PgPool) {
    seed_with_status(&pool, "task-s", TaskStatus::Closed).await;
    let state = test_app_state(pool.clone());

    let event = terminal_event("evt-s", "task-s", "stopped", serde_json::json!({}));
    handle_agent_stopped(&state, &event).await;

    assert_eq!(status_of(&pool, "task-s").await, TaskStatus::Closed);
}

#[sqlx::test]
async fn error_does_not_reopen_closed_task(pool: PgPool) {
    seed_with_status(&pool, "task-e", TaskStatus::Closed).await;
    let state = test_app_state(pool.clone());

    let event = terminal_event(
        "evt-e",
        "task-e",
        "error",
        serde_json::json!({ "error": "boom" }),
    );
    handle_agent_error(&state, &event).await;

    assert_eq!(status_of(&pool, "task-e").await, TaskStatus::Closed);
}

#[sqlx::test]
async fn completed_reopens_an_open_task(pool: PgPool) {
    // Control: the same handler still (re)opens a non-closed task.
    seed_with_status(&pool, "task-o", TaskStatus::Error).await;
    let state = test_app_state(pool.clone());

    let event = terminal_event(
        "evt-o",
        "task-o",
        "completed",
        serde_json::json!({ "is_error": false }),
    );
    handle_agent_completed(&state, &event).await;

    assert_eq!(status_of(&pool, "task-o").await, TaskStatus::Open);
}

// =========================================================================
// Orphaned-turn reconcile regression
// =========================================================================

#[sqlx::test]
async fn reconcile_error_keeps_closed_but_appends_message(pool: PgPool) {
    // Mirrors `reaper::close_orphaned_turn`: a synthetic agent.error on a closed
    // task must keep it closed, yet still append the error message row so the UI
    // turn unblocks.
    seed_with_status(&pool, "task-r", TaskStatus::Closed).await;
    let state = test_app_state(pool.clone());

    let event = terminal_event(
        "reconcile-task-r-1",
        "task-r",
        "error",
        serde_json::json!({
            "error": "The agent stopped without reporting a result.",
            "reason": "reconciled_orphaned_turn",
        }),
    );
    handle_agent_error(&state, &event).await;

    assert_eq!(status_of(&pool, "task-r").await, TaskStatus::Closed);

    let (rows, _, _) = moatless_vibe_backend::message::dao::get_all(&pool, "task-r")
        .await
        .unwrap();
    assert!(
        rows.iter().any(|m| m
            .message_content
            .to_string()
            .contains("stopped without reporting a result")),
        "the synthetic error message row should still be appended"
    );
}

// =========================================================================
// Auto-provision gate (Fix B) — direct fix for the "close started it again" race
// =========================================================================

#[sqlx::test]
async fn build_provision_request_rejects_closed_task(pool: PgPool) {
    use moatless_vibe_backend::auth::provider_token_dao::PgProviderTokenRepository;
    use moatless_vibe_backend::plugin_repo::dao::PgPluginRepoRepository;
    use moatless_vibe_backend::repository::dao::PgRepoRepository;
    use moatless_vibe_backend::sandbox::auto_provision::build_provision_request;

    seed_with_status(&pool, "task-prov", TaskStatus::Closed).await;

    let result = build_provision_request(
        &PgTaskRepository(pool.clone()),
        &PgRepoRepository(pool.clone()),
        &PgProviderTokenRepository(pool.clone()),
        &PgPluginRepoRepository(pool.clone()),
        "task-prov",
    )
    .await;

    assert!(
        result.is_err(),
        "a closed task must not be provisioned — a webhook racing a close is rejected"
    );
}
