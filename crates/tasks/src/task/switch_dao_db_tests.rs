//! PostgreSQL tests for the profile switch's transaction.

use sqlx::PgPool;

use crate::task::switch_dao::write_profile;
use crate::task::test_support::{make_task_row, seed_task};
use crate::task_event::models::InsertTaskEvent;
use moatless_types::agent::AgentProfile;
use moatless_workspaces::test_support::make_repository_row;

async fn seed_a_task(pool: &PgPool, task_id: &str) {
    let repo = make_repository_row("repo-1", "user-1");
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    seed_task(pool, &make_task_row(task_id, "user-1"))
        .await
        .expect("seed task");
}

async fn listing_changes(pool: &PgPool, task_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM task_listing_changes WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .expect("read the listing changes")
}

async fn activities(pool: &PgPool, task_id: &str) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT kind, summary FROM conversation_activities \
          WHERE task_id = $1 ORDER BY position, ordinal",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .expect("read activities")
}

fn switch_note(task_id: &str, summary: &str) -> InsertTaskEvent {
    InsertTaskEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.to_string(),
        event_type: "task".to_string(),
        sub_type: "agent_switched".to_string(),
        is_error: false,
        agent_profile: Some("claude-code".to_string()),
        turn_number: Some(1),
        payload: serde_json::json!({ "summary": summary, "from": "codex", "to": "claude-code" }),
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    }
}

/// The T3 shell feed re-reads only the rows the listing names, so a switch
/// that records no change leaves the thread on its old model.
#[sqlx::test]
async fn a_switch_moves_the_listing_row(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    let seeded = listing_changes(&pool, "task-1").await;

    write_profile(
        &pool,
        "task-1",
        AgentProfile::ClaudeCode,
        Some("opus"),
        None,
    )
    .await
    .expect("switch");

    assert_eq!(listing_changes(&pool, "task-1").await, seeded + 1);
    let (profile, model): (String, Option<String>) =
        sqlx::query_as("SELECT agent_profile, model FROM tasks WHERE task_id = $1")
            .bind("task-1")
            .fetch_one(&pool)
            .await
            .expect("read the task");
    assert_eq!(profile, "claude-code");
    assert_eq!(model.as_deref(), Some("opus"));
    assert!(activities(&pool, "task-1").await.is_empty());
}

/// A fallback's note is projected into the conversation with the switch, so
/// the thread shows why it changed agent.
#[sqlx::test]
async fn a_fallback_switch_projects_its_reason_into_the_conversation(pool: PgPool) {
    seed_a_task(&pool, "task-1").await;
    let summary = "Switched from Codex to Claude Code: Codex hit its usage limit";

    write_profile(
        &pool,
        "task-1",
        AgentProfile::ClaudeCode,
        None,
        Some(&switch_note("task-1", summary)),
    )
    .await
    .expect("switch");

    assert_eq!(
        activities(&pool, "task-1").await,
        vec![("agent_switched".to_string(), summary.to_string())]
    );
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_events WHERE task_id = $1 AND sub_type = 'agent_switched'",
    )
    .bind("task-1")
    .fetch_one(&pool)
    .await
    .expect("read the events");
    assert_eq!(events, 1);
}
