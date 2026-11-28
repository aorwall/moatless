#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration fixtures fail on invalid setup"
)]
//! The reaper's stranded-inbox pass re-wakes a parent whose child notice was
//! queued but never opened a turn.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use moatless_backend::event_handler::agent_handlers::handle_agent_completed;
use moatless_backend::reaper_stranded::rewake_stranded_inboxes;
use moatless_ports::sandbox::SandboxWaker;
use moatless_tasks::agent_event::AgentEvent;
use moatless_tasks::task::relations::{TaskRelation, TaskRelationType};
use moatless_tasks::task::test_support as task_fixtures;
use moatless_workspaces::compose;
use sqlx::PgPool;

use crate::common::{make_repository_row, make_task_row, test_app_state};

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

async fn seed_parent_and_child(pool: &PgPool) {
    compose::seed_repository(pool, &make_repository_row("repo-1", "user-1"))
        .await
        .unwrap();
    for task_id in ["task-parent", "task-child"] {
        task_fixtures::seed_task(pool, &make_task_row(task_id, "user-1"))
            .await
            .unwrap();
    }
    let relation = TaskRelation::new(
        "task-child",
        "task-parent",
        TaskRelationType::CreatedBy,
        serde_json::json!({}),
    );
    task_fixtures::seed_relation(pool, &relation).await.unwrap();
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, triggered_by_task_id, trigger_decided) \
         VALUES ('task-child', 1, 'task-parent', true)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE sandboxes SET desired_state = 'running' WHERE sandbox_id = 'task-parent'")
        .execute(pool)
        .await
        .unwrap();
}

fn child_completed() -> AgentEvent {
    serde_json::from_value(serde_json::json!({
        "eventId": "evt-child-completed",
        "type": "agent",
        "subType": "completed",
        "taskId": "task-child",
        "timestamp": "2026-11-10T13:27:55.000Z",
        "isError": false,
        "agentProfile": "claude-code",
        "turnNumber": 1,
        "payload": { "is_error": false },
    }))
    .unwrap()
}

/// Regression: an orchestrator got its child's notice and no turn ran for two
/// days, because nothing looked again at a wake that was lost.
#[sqlx::test]
async fn a_child_notice_no_turn_took_wakes_the_parent_again(pool: PgPool) {
    seed_parent_and_child(&pool).await;
    let mut state = test_app_state(pool.clone());
    let first_wake = Arc::new(RecordingWaker::default());
    state.sandbox_waker = first_wake.clone();
    handle_agent_completed(&state, &child_completed()).await;
    assert_eq!(*first_wake.woken.lock().unwrap(), ["task-parent"]);

    let reaper_waker = RecordingWaker::default();
    let rewoken = Mutex::new(HashMap::new());
    let now = Utc::now();
    rewake_stranded_inboxes(&state.messages, &reaper_waker, &rewoken, now).await;
    assert!(
        reaper_waker.woken.lock().unwrap().is_empty(),
        "a notice sent moments ago is still on its way"
    );

    let later = now + Duration::minutes(30);
    rewake_stranded_inboxes(&state.messages, &reaper_waker, &rewoken, later).await;
    rewake_stranded_inboxes(&state.messages, &reaper_waker, &rewoken, later).await;
    assert_eq!(
        *reaper_waker.woken.lock().unwrap(),
        ["task-parent"],
        "woken once per spacing, not on every cycle"
    );

    rewake_stranded_inboxes(
        &state.messages,
        &reaper_waker,
        &rewoken,
        later + Duration::minutes(15),
    )
    .await;
    assert_eq!(
        *reaper_waker.woken.lock().unwrap(),
        ["task-parent", "task-parent"]
    );
}
