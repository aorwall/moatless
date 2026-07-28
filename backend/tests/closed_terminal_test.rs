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
use moatless_vibe_backend::message::dao as message_dao;
use moatless_vibe_backend::message::models::{MessageRole, MessageType};
use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::task::dao::{
    self as task_dao, PgTaskRepository, TaskPatch, TaskRepository,
};
use moatless_vibe_backend::task::models::TaskStatus;
use moatless_vibe_backend::task::relations::{
    PgTaskRelationRepository, TaskRelation, TaskRelationRepository, TaskRelationType,
};
use sqlx::PgPool;

use common::{make_message_row, make_repository_row, make_task_row, test_app_state, user_scope};

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
                &user_scope("user-1"),
            )
            .await
            .unwrap();
    }
}

async fn seed_task_pair_with_created_by(pool: &PgPool, parent_task_id: &str, child_task_id: &str) {
    let repo = make_repository_row("repo-1", "user-1");
    repository_dao::insert(pool, &repo).await.unwrap();
    task_dao::insert_task(pool, &make_task_row(parent_task_id, "user-1"))
        .await
        .unwrap();
    task_dao::insert_task(pool, &make_task_row(child_task_id, "user-1"))
        .await
        .unwrap();

    let relation = TaskRelation::new(
        child_task_id,
        parent_task_id,
        TaskRelationType::CreatedBy,
        serde_json::json!({}),
    );
    PgTaskRelationRepository(pool.clone())
        .insert(&relation)
        .await
        .unwrap();
}

async fn status_of(pool: &PgPool, task_id: &str) -> TaskStatus {
    task_dao::find_by_id(pool, task_id, &user_scope("user-1"))
        .await
        .unwrap()
        .expect("task present")
        .status
}

async fn visible_parent_texts(pool: &PgPool, parent_task_id: &str) -> Vec<String> {
    let (messages, _, blocks) = message_dao::get_all(pool, parent_task_id).await.unwrap();
    let user_ids: std::collections::HashSet<i32> = messages
        .iter()
        .filter(|m| m.message_type == MessageType::User)
        .map(|m| m.id)
        .collect();

    blocks
        .into_iter()
        .filter(|b| user_ids.contains(&b.task_message_id) && b.show_in_ui)
        .filter_map(|b| b.text_content)
        .collect()
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
        .update_status_if_not_closed("task-closed", TaskStatus::Open, &user_scope("user-1"))
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
        .update_status_if_not_closed("task-open", TaskStatus::Open, &user_scope("user-1"))
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
            &user_scope("user-1"),
        )
        .await
        .unwrap();
    assert!(repo
        .update_status_if_not_closed("task-open", TaskStatus::Open, &user_scope("user-1"))
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

#[sqlx::test]
async fn completed_child_notifies_parent_without_result_body(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    let mut assistant = make_message_row("task-child", "child-assistant-1");
    assistant.message_type = MessageType::Assistant;
    assistant.message_role = MessageRole::Assistant;
    assistant.message_content = serde_json::json!([
        { "type": "text", "text": "full child result that should stay out of parent" }
    ]);
    message_dao::append(&pool, &assistant, &[], &[])
        .await
        .unwrap();

    let state = test_app_state(pool.clone());
    let event = terminal_event(
        "evt-child-completed",
        "task-child",
        "completed",
        serde_json::json!({ "is_error": false }),
    );
    handle_agent_completed(&state, &event).await;

    let texts = visible_parent_texts(&pool, "task-parent").await;
    assert!(
        texts
            .iter()
            .any(|text| text == "Child task `task-child` completed."),
        "parent should receive a concise child completion notice: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .all(|text| !text.contains("full child result that should stay out of parent")),
        "parent notification must not include the child's full assistant result: {texts:?}"
    );
}

#[sqlx::test]
async fn errored_child_notifies_parent_with_error_message(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    let state = test_app_state(pool.clone());

    let event = terminal_event(
        "evt-child-error",
        "task-child",
        "error",
        serde_json::json!({ "error": "sandbox exploded" }),
    );
    handle_agent_error(&state, &event).await;

    let texts = visible_parent_texts(&pool, "task-parent").await;
    assert!(
        texts
            .iter()
            .any(|text| text == "Child task `task-child` errored.\n\nError: sandbox exploded"),
        "parent should receive the child error detail: {texts:?}"
    );
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
