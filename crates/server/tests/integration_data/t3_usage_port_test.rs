#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for the `UsageReporter` port the T3 band builds.
//!
//! The bucket query is built by string interpolation and bound positionally, so
//! a parameter index that drifts from the bind order compiles and only fails
//! when Postgres prepares the statement. Every test here executes it.

use crate::common;

use moatless_t3code::ports::UsageWindow;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_workspaces::compose;
use serde_json::json;
use sqlx::PgPool;

use common::{USER_1, make_repository_row, make_task_row, seed_team, user_scope};

/// Seed the team + repository a task's FKs require, then the task itself.
async fn seed_task(pool: &PgPool, task_id: &str, user_id: &str) {
    seed_team(pool, &format!("personal-{user_id}")).await;
    let repo = make_repository_row("repo-1", user_id);
    let _ = compose::seed_repository(pool, &repo).await;
    let task = make_task_row(task_id, user_id);
    task_fixtures::seed_task(pool, &task)
        .await
        .expect("seed task row");
}

/// Write one completed turn whose `started_at` lands on 2026-09-02 in UTC.
async fn seed_turn(
    pool: &PgPool,
    task_id: &str,
    turn_number: i32,
    agent_type: &str,
    metrics: serde_json::Value,
) {
    sqlx::query("UPDATE tasks SET agent_type = $2 WHERE task_id = $1")
        .bind(task_id)
        .bind(agent_type)
        .execute(pool)
        .await
        .expect("set task harness");
    sqlx::query(
        r#"INSERT INTO task_turns (task_id, turn_number, status, agent_type, started_at, metrics)
           VALUES ($1, $2, 'completed', $3, '2026-09-02T10:00:00Z'::timestamptz, $4)"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .bind(agent_type)
    .bind(metrics)
    .execute(pool)
    .await
    .expect("seed turn row");
}

#[sqlx::test]
async fn global_usage_respects_local_day_and_missing_metric_fields(pool: PgPool) {
    seed_task(&pool, "global-task", USER_1).await;
    seed_turn(
        &pool,
        "global-task",
        1,
        "codex",
        json!({"usage": {"input_tokens": 7}}),
    )
    .await;
    seed_turn(&pool, "global-task", 2, "codex", json!({})).await;
    sqlx::query(
        "UPDATE tasks SET scope = 'global', owner_user_id = NULL WHERE task_id = 'global-task'",
    )
    .execute(&pool)
    .await
    .expect("global task");
    sqlx::query("UPDATE task_turns SET started_at = '2026-09-02T00:30:00Z'::timestamptz WHERE task_id = 'global-task'")
        .execute(&pool).await.expect("turn near day boundary");
    let reporter = moatless_backend::t3code::t3_graph_over(&pool).usage();
    let viewer = user_scope("22222222-2222-2222-2222-222222222222");
    let local_window = UsageWindow {
        since_day: "2026-09-01".into(),
        until_day: "2026-09-01".into(),
        time_zone: "America/Los_Angeles".into(),
        hourly: true,
    };
    let rows = reporter
        .buckets(&viewer, &local_window)
        .await
        .expect("local buckets");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].day, "2026-09-01");
    assert_eq!(rows[0].hour_start.as_deref(), Some("2026-09-01T17:00:00Z"));
    assert_eq!(rows[0].model, "unknown");
    assert_eq!(rows[0].records, 1);
    assert_eq!(rows[0].unpriced_records, 1);
    assert_eq!(rows[0].uncached_input_tokens, 7);
    assert_eq!(rows[0].output_tokens, 0);
    assert_eq!(rows[0].sessions, 1);
    let utc_window = UsageWindow {
        time_zone: "UTC".into(),
        ..local_window
    };
    assert!(
        reporter
            .buckets(&viewer, &utc_window)
            .await
            .expect("UTC excludes prior local day")
            .is_empty()
    );
}

fn window() -> UsageWindow {
    UsageWindow {
        since_day: "2026-09-01".to_string(),
        until_day: "2026-09-03".to_string(),
        time_zone: "UTC".to_string(),
        hourly: false,
    }
}

#[sqlx::test]
async fn claude_turn_fans_out_one_bucket_per_model(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;
    seed_turn(
        &pool,
        "task-1",
        1,
        "claude-code",
        json!({
            "modelUsage": {
                "claude-opus-5": {
                    "inputTokens": 100,
                    "cacheReadInputTokens": 900,
                    "cacheCreationInputTokens": 40,
                    "outputTokens": 300,
                    "costUSD": 1.25,
                },
                "claude-haiku-4-5": {
                    "inputTokens": 10,
                    "cacheReadInputTokens": 0,
                    "cacheCreationInputTokens": 0,
                    "outputTokens": 5,
                    "costUSD": 0.01,
                },
            }
        }),
    )
    .await;

    let reporter = moatless_backend::t3code::t3_graph_over(&pool).usage();
    let rows = reporter
        .buckets(&user_scope(USER_1), &window())
        .await
        .expect("read usage buckets");

    assert_eq!(rows.len(), 2, "one bucket per model: {rows:?}");
    let opus = rows
        .iter()
        .find(|row| row.model == "claude-opus-5")
        .expect("opus bucket");
    assert_eq!(opus.day, "2026-09-02");
    assert_eq!(opus.hour_start, None);
    assert_eq!(opus.provider, "claude");
    assert_eq!(opus.uncached_input_tokens, 100);
    assert_eq!(opus.cached_input_tokens, 900);
    assert_eq!(opus.cache_creation_tokens, 40);
    assert_eq!(opus.output_tokens, 300);
    assert!((opus.cost_usd - 1.25).abs() < 1e-9);
    assert_eq!(opus.records, 1);
    assert_eq!(opus.unpriced_records, 0);
    assert_eq!(opus.sessions, 1);
}

#[sqlx::test]
async fn codex_turn_groups_by_its_single_model(pool: PgPool) {
    seed_task(&pool, "task-2", USER_1).await;
    seed_turn(
        &pool,
        "task-2",
        1,
        "codex",
        json!({
            "model": "gpt-5-codex",
            "totalCostUsd": 0.5,
            "usage": {
                "input_tokens": 2000,
                "cache_read_input_tokens": 1500,
                "cache_creation_input_tokens": 0,
                "output_tokens": 400,
                "reasoning_output_tokens": 120,
            }
        }),
    )
    .await;

    let reporter = moatless_backend::t3code::t3_graph_over(&pool).usage();
    let rows = reporter
        .buckets(&user_scope(USER_1), &window())
        .await
        .expect("read usage buckets");

    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].provider, "codex");
    assert_eq!(rows[0].model, "gpt-5-codex");
    assert_eq!(rows[0].reasoning_tokens, 120);
    assert!((rows[0].cost_usd - 0.5).abs() < 1e-9);
}

#[sqlx::test]
async fn hourly_window_carries_the_hour_and_a_foreign_viewer_reads_nothing(pool: PgPool) {
    seed_task(&pool, "task-3", USER_1).await;
    seed_turn(
        &pool,
        "task-3",
        1,
        "claude-code",
        json!({"modelUsage": {"claude-opus-5": {"inputTokens": 1, "outputTokens": 2}}}),
    )
    .await;

    let reporter = moatless_backend::t3code::t3_graph_over(&pool).usage();
    let hourly = UsageWindow {
        hourly: true,
        ..window()
    };
    let rows = reporter
        .buckets(&user_scope(USER_1), &hourly)
        .await
        .expect("read usage buckets");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].hour_start.as_deref(), Some("2026-09-02T10:00:00Z"));
    assert_eq!(
        rows[0].unpriced_records, 1,
        "no costUSD on the model entry makes the record unpriced"
    );

    let stranger = user_scope("22222222-2222-2222-2222-222222222222");
    let hidden = reporter
        .buckets(&stranger, &window())
        .await
        .expect("read usage buckets");
    assert!(hidden.is_empty(), "{hidden:?}");
}
