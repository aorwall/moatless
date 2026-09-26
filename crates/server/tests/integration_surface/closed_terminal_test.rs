#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for "Closed is terminal against automated re-open" (Fix
//! A).
//!
//! Covers:
//! - `TaskDao::update_status_if_not_closed` — race-safe conditional flip.
//! - The three agent terminal handlers (`completed` / `stopped` / `error`) do
//!   not resurrect a `Closed` task to `Open`.
//! - A `created_by` child's completion or error notices its parent once.
//! - Regression: see `reconcile_error_keeps_closed_and_says_nothing`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::common;

use moatless_backend::event_handler::agent_handlers::{
    handle_agent_completed, handle_agent_error, handle_agent_stopped,
};
use moatless_backend::state::AppState;
use moatless_ports::sandbox::SandboxWaker;
use moatless_tasks::agent_event::AgentEvent;
use moatless_tasks::message::models::MessageRole;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::test_support as message_fixtures;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::relations::TaskRelation;
use moatless_tasks::task::relations::TaskRelationType;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_tasks::task_activities::mapping::ConversationWrite;
use moatless_workspaces::compose;
use sqlx::PgPool;

use common::{make_message_row, make_repository_row, make_task_row, test_app_state, user_scope};

/// Seed a repo + task with the given status and return an `AppState`.
async fn seed_with_status(pool: &PgPool, task_id: &str, status: TaskStatus) {
    let repo = make_repository_row("repo-1", "user-1");
    compose::seed_repository(pool, &repo).await.unwrap();
    let task = make_task_row(task_id, "user-1");
    task_fixtures::seed_task(pool, &task).await.unwrap();
    if status != TaskStatus::Open {
        moatless_tasks::task::test_support::patch_task(
            pool,
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
    compose::seed_repository(pool, &repo).await.unwrap();
    task_fixtures::seed_task(pool, &make_task_row(parent_task_id, "user-1"))
        .await
        .unwrap();
    task_fixtures::seed_task(pool, &make_task_row(child_task_id, "user-1"))
        .await
        .unwrap();

    let relation = TaskRelation::new(
        child_task_id,
        parent_task_id,
        TaskRelationType::CreatedBy,
        serde_json::json!({}),
    );
    moatless_tasks::task::test_support::seed_relation(pool, &relation)
        .await
        .unwrap();
}

/// Open the child's turn as one its parent's message triggered.
async fn seed_turn_triggered_by(pool: &PgPool, task_id: &str, turn_number: i32, trigger: &str) {
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, triggered_by_task_id, trigger_decided) \
         VALUES ($1, $2, $3, true)",
    )
    .bind(task_id)
    .bind(turn_number)
    .bind(trigger)
    .execute(pool)
    .await
    .unwrap();
}

async fn status_of(pool: &PgPool, task_id: &str) -> TaskStatus {
    task_fixtures::find_task(pool, task_id, &user_scope("user-1"))
        .await
        .unwrap()
        .expect("task present")
        .status
}

async fn visible_parent_texts(pool: &PgPool, parent_task_id: &str) -> Vec<String> {
    let (messages, _, _) = message_fixtures::messages_for(pool, parent_task_id)
        .await
        .unwrap();
    messages
        .into_iter()
        .filter(|m| m.message_type == MessageType::User)
        .filter_map(|m| m.message_text)
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
        "agentProfile": "claude-code",
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

    let changed = moatless_tasks::task::test_support::set_status_if_open(
        &pool,
        "task-closed",
        TaskStatus::Open,
        &user_scope("user-1"),
    )
    .await
    .unwrap();

    assert!(!changed, "a closed task must not be flipped");
    assert_eq!(status_of(&pool, "task-closed").await, TaskStatus::Closed);
}

#[sqlx::test]
async fn update_status_if_not_closed_flips_open_and_error(pool: PgPool) {
    // Open → Open still matches the predicate (status <> 'closed').
    seed_with_status(&pool, "task-open", TaskStatus::Open).await;
    assert!(
        moatless_tasks::task::test_support::set_status_if_open(
            &pool,
            "task-open",
            TaskStatus::Open,
            &user_scope("user-1"),
        )
        .await
        .unwrap()
    );
    assert_eq!(status_of(&pool, "task-open").await, TaskStatus::Open);

    // Error → Open flips (preserves the start_task-from-Error path).
    moatless_tasks::task::test_support::patch_task(
        &pool,
        "task-open",
        &TaskPatch {
            status: Some(TaskStatus::Error),
            ..Default::default()
        },
        &user_scope("user-1"),
    )
    .await
    .unwrap();
    assert!(
        moatless_tasks::task::test_support::set_status_if_open(
            &pool,
            "task-open",
            TaskStatus::Open,
            &user_scope("user-1"),
        )
        .await
        .unwrap()
    );
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
    handle_agent_error(&(&state).into(), &event).await;

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

/// Records the task ids it is woken with.
#[derive(Default)]
struct RecordingWaker {
    woken: Mutex<Vec<String>>,
}

#[async_trait]
impl SandboxWaker for RecordingWaker {
    async fn wake(&self, task_id: &str) -> Result<(), String> {
        self.woken.lock().unwrap().push(task_id.to_string());
        Ok(())
    }
}

fn state_with_recording_waker(pool: &PgPool) -> (AppState, Arc<RecordingWaker>) {
    let waker = Arc::new(RecordingWaker::default());
    let mut state = test_app_state(pool.clone());
    state.sandbox_waker = waker.clone();
    (state, waker)
}

fn completed_event(event_id: &str, task_id: &str, turn_number: i32) -> AgentEvent {
    let mut event = terminal_event(
        event_id,
        task_id,
        "completed",
        serde_json::json!({ "is_error": false }),
    );
    event.turn_number = Some(turn_number);
    event
}

fn completion_notice(turn_number: i32) -> String {
    format!(
        "Child task `task-child` (Test task task-child) completed turn {turn_number}. \
         This notice is automatic and carries no result; read the child's conversation \
         if you need one. It needs no reply."
    )
}

#[sqlx::test]
async fn completed_child_sends_parent_one_notice_and_one_wake(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    seed_turn_triggered_by(&pool, "task-child", 1, "task-parent").await;
    let mut assistant = make_message_row("task-child", "child-assistant-1");
    assistant.message_type = MessageType::Assistant;
    assistant.message_role = MessageRole::Assistant;
    assistant.message_text = Some("full child result that should stay out of parent".to_string());
    message_fixtures::seed_message(&pool, &assistant, &[], &ConversationWrite::default())
        .await
        .unwrap();
    let (state, waker) = state_with_recording_waker(&pool);

    handle_agent_completed(
        &state,
        &completed_event("evt-child-completed", "task-child", 1),
    )
    .await;

    assert_eq!(
        visible_parent_texts(&pool, "task-parent").await,
        vec![completion_notice(1)]
    );
    assert_eq!(
        *waker.woken.lock().unwrap(),
        vec!["task-parent".to_string()]
    );
    assert!(
        visible_parent_texts(&pool, "task-child").await.is_empty(),
        "the notice sets nothing off in the child"
    );
}

#[sqlx::test]
async fn redelivered_completion_adds_no_second_notice_but_a_new_turn_does(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    seed_turn_triggered_by(&pool, "task-child", 1, "task-parent").await;
    seed_turn_triggered_by(&pool, "task-child", 2, "task-parent").await;
    let (state, waker) = state_with_recording_waker(&pool);

    let first = completed_event("evt-turn-1", "task-child", 1);
    handle_agent_completed(&state, &first).await;
    // A rename between deliveries changes the notice text, not its identity.
    moatless_tasks::task::test_support::patch_task(
        &pool,
        "task-child",
        &TaskPatch {
            name: Some("Renamed child".to_string()),
            ..Default::default()
        },
        &user_scope("user-1"),
    )
    .await
    .unwrap();
    handle_agent_completed(&state, &first).await;
    handle_agent_completed(&state, &completed_event("evt-turn-2", "task-child", 2)).await;

    assert_eq!(
        visible_parent_texts(&pool, "task-parent").await,
        vec![
            completion_notice(1),
            completion_notice(2).replace("Test task task-child", "Renamed child"),
        ]
    );
    assert_eq!(
        *waker.woken.lock().unwrap(),
        vec!["task-parent".to_string(), "task-parent".to_string()],
        "the redelivery wakes nobody"
    );
    let queued = message_fixtures::pending_user_messages(&pool, "task-parent")
        .await
        .unwrap();
    let senders: Vec<Option<&str>> = queued.iter().map(|m| m.source_task_id.as_deref()).collect();
    assert_eq!(
        senders,
        [Some("task-child"), Some("task-child")],
        "one inbox item per turn, each from the child"
    );
}

#[sqlx::test]
async fn a_turn_leaving_background_commands_defers_the_notice_to_the_wake_turn(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    seed_turn_triggered_by(&pool, "task-child", 1, "task-parent").await;
    seed_turn_triggered_by(&pool, "task-child", 2, "task-parent").await;
    let (state, waker) = state_with_recording_waker(&pool);

    let mut first = completed_event("evt-turn-1", "task-child", 1);
    first.payload = serde_json::json!({ "is_error": false, "backgroundCommands": 1 });
    handle_agent_completed(&state, &first).await;

    assert!(visible_parent_texts(&pool, "task-parent").await.is_empty());
    assert!(waker.woken.lock().unwrap().is_empty());

    handle_agent_completed(&state, &completed_event("evt-turn-2", "task-child", 2)).await;

    assert_eq!(
        visible_parent_texts(&pool, "task-parent").await,
        vec![completion_notice(2)]
    );
    assert_eq!(
        *waker.woken.lock().unwrap(),
        vec!["task-parent".to_string()]
    );
}

async fn desired_state_of(pool: &PgPool, sandbox_id: &str) -> String {
    sqlx::query_scalar("SELECT desired_state FROM sandboxes WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn a_turn_leaving_background_commands_keeps_a_stop_when_done_sandbox(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    sqlx::query("UPDATE sandboxes SET stop_when_done = true WHERE sandbox_id = 'task-child'")
        .execute(&pool)
        .await
        .unwrap();
    let before = desired_state_of(&pool, "task-child").await;
    let state = test_app_state(pool.clone());

    let mut first = completed_event("evt-turn-1", "task-child", 1);
    first.payload = serde_json::json!({ "is_error": false, "backgroundCommands": 1 });
    handle_agent_completed(&state, &first).await;
    assert_eq!(desired_state_of(&pool, "task-child").await, before);

    handle_agent_completed(&state, &completed_event("evt-turn-2", "task-child", 2)).await;
    assert_eq!(desired_state_of(&pool, "task-child").await, "stopped");
}

#[sqlx::test]
async fn completed_fork_notifies_nobody(pool: PgPool) {
    let repo = make_repository_row("repo-1", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    for task_id in ["task-source", "task-fork"] {
        task_fixtures::seed_task(&pool, &make_task_row(task_id, "user-1"))
            .await
            .unwrap();
    }
    let relation = TaskRelation::new(
        "task-fork",
        "task-source",
        TaskRelationType::ForkedFrom,
        serde_json::json!({}),
    );
    moatless_tasks::task::test_support::seed_relation(&pool, &relation)
        .await
        .unwrap();
    let (state, waker) = state_with_recording_waker(&pool);

    handle_agent_completed(&state, &completed_event("evt-fork", "task-fork", 1)).await;

    assert!(visible_parent_texts(&pool, "task-source").await.is_empty());
    assert!(waker.woken.lock().unwrap().is_empty());
}

#[sqlx::test]
async fn completed_child_of_a_closed_parent_notifies_nobody(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    seed_turn_triggered_by(&pool, "task-child", 1, "task-parent").await;
    moatless_tasks::task::test_support::patch_task(
        &pool,
        "task-parent",
        &TaskPatch {
            status: Some(TaskStatus::Closed),
            ..Default::default()
        },
        &user_scope("user-1"),
    )
    .await
    .unwrap();
    let (state, waker) = state_with_recording_waker(&pool);

    handle_agent_completed(&state, &completed_event("evt-closed", "task-child", 1)).await;

    assert!(visible_parent_texts(&pool, "task-parent").await.is_empty());
    assert!(waker.woken.lock().unwrap().is_empty());
}

#[sqlx::test]
async fn stopped_child_notifies_nobody(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    let (state, waker) = state_with_recording_waker(&pool);

    let event = terminal_event("evt-stop", "task-child", "stopped", serde_json::json!({}));
    handle_agent_stopped(&state, &event).await;

    assert!(visible_parent_texts(&pool, "task-parent").await.is_empty());
    assert!(waker.woken.lock().unwrap().is_empty());
}

#[sqlx::test]
async fn a_turn_the_parent_did_not_trigger_notifies_nobody(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    sqlx::query("INSERT INTO task_turns (task_id, turn_number) VALUES ('task-child', 1)")
        .execute(&pool)
        .await
        .unwrap();
    seed_turn_triggered_by(&pool, "task-child", 2, "task-other").await;
    let (state, waker) = state_with_recording_waker(&pool);

    handle_agent_completed(&state, &completed_event("evt-person", "task-child", 1)).await;
    handle_agent_completed(&state, &completed_event("evt-other", "task-child", 2)).await;
    let mut errored = terminal_event(
        "evt-person-error",
        "task-child",
        "error",
        serde_json::json!({ "error": "sandbox exploded" }),
    );
    errored.turn_number = Some(1);
    handle_agent_error(&(&state).into(), &errored).await;

    assert!(visible_parent_texts(&pool, "task-parent").await.is_empty());
    assert!(waker.woken.lock().unwrap().is_empty());
}

const ERROR_NOTICE: &str = "Child task `task-child` errored.\n\nError: sandbox exploded";

#[sqlx::test]
async fn errored_child_sends_parent_only_the_error_notice(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    seed_turn_triggered_by(&pool, "task-child", 1, "task-parent").await;
    let (state, waker) = state_with_recording_waker(&pool);

    let event = terminal_event(
        "evt-child-error",
        "task-child",
        "error",
        serde_json::json!({ "error": "sandbox exploded" }),
    );
    handle_agent_error(&(&state).into(), &event).await;

    assert_eq!(
        visible_parent_texts(&pool, "task-parent").await,
        vec![ERROR_NOTICE.to_string()]
    );
    assert_eq!(
        *waker.woken.lock().unwrap(),
        vec!["task-parent".to_string()]
    );
}

/// One Sandbox failure publishes `pod_evicted` and then `fatal`, and both reach
/// `handle_agent_error`; the parent is told once per child turn.
#[sqlx::test]
async fn a_second_error_in_one_turn_sends_no_second_notice(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    seed_turn_triggered_by(&pool, "task-child", 1, "task-parent").await;
    let (state, waker) = state_with_recording_waker(&pool);

    for (event_id, error) in [("evt-evicted", "sandbox exploded"), ("evt-fatal", "again")] {
        let mut event = terminal_event(
            event_id,
            "task-child",
            "error",
            serde_json::json!({ "error": error }),
        );
        event.turn_number = Some(1);
        handle_agent_error(&(&state).into(), &event).await;
    }

    assert_eq!(
        visible_parent_texts(&pool, "task-parent").await,
        vec![ERROR_NOTICE.to_string()]
    );
    assert_eq!(
        *waker.woken.lock().unwrap(),
        vec!["task-parent".to_string()]
    );
}

#[sqlx::test]
async fn an_error_result_under_completed_sends_only_the_error_notice(pool: PgPool) {
    seed_task_pair_with_created_by(&pool, "task-parent", "task-child").await;
    seed_turn_triggered_by(&pool, "task-child", 1, "task-parent").await;
    let (state, waker) = state_with_recording_waker(&pool);

    let event = terminal_event(
        "evt-child-error-result",
        "task-child",
        "completed",
        serde_json::json!({ "is_error": true, "error": "sandbox exploded" }),
    );
    handle_agent_completed(&state, &event).await;

    assert_eq!(
        visible_parent_texts(&pool, "task-parent").await,
        vec![ERROR_NOTICE.to_string()]
    );
    assert_eq!(
        *waker.woken.lock().unwrap(),
        vec!["task-parent".to_string()]
    );
}

// =========================================================================
// Orphaned-turn reconcile regression
// =========================================================================

#[sqlx::test]
async fn reconcile_error_keeps_closed_and_says_nothing(pool: PgPool) {
    // Mirrors `reaper::close_orphaned_turn`: a synthetic agent.error on a closed
    // task must keep it closed. It writes no conversation row — a run outcome is
    // not a Message (Decision 14); the turn's own status is what unblocks the UI.
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
    handle_agent_error(&(&state).into(), &event).await;

    assert_eq!(status_of(&pool, "task-r").await, TaskStatus::Closed);

    let (rows, _, _) = moatless_tasks::message::test_support::messages_for(&pool, "task-r")
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "a reconciled orphaned turn is not something anybody said: {rows:?}"
    );
}
