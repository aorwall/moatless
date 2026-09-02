#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for task_turn_dao — requires `DATABASE_URL` and
//! `--features db-tests`. Covers the transactional ingress writer, the
//! idempotent consumer-fallback projection, the reaper's indexed list query,
//! and the rollout backfill from the event log.

use crate::common;

use chrono::{Duration, Utc};
use moatless_backend::task::dao::{self as task_dao};
use moatless_backend::task::models::TaskStatus;
use moatless_backend::task_activities::mapping::ConversationWrite;
use moatless_backend::task_turn::dao::{PgTaskTurnDao, TaskTurnDao, TurnStateEvent};
use moatless_backend::task_turn::models::{TurnSignals, TurnStatus, TurnTransition};
use sqlx::PgPool;

use common::{make_repository_row, make_task_row};

/// Read back a `task_turns` row as `(status, has_ended_at, agent_type)`.
async fn turn_row(
    pool: &PgPool,
    task_id: &str,
    turn_number: i32,
) -> Option<(String, bool, Option<String>)> {
    let row: Option<(String, Option<chrono::DateTime<Utc>>, Option<String>)> = sqlx::query_as(
        r#"SELECT status, ended_at, agent_type
           FROM task_turns WHERE task_id = $1 AND turn_number = $2"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .fetch_optional(pool)
    .await
    .expect("read turn");
    row.map(|(status, ended_at, agent_type)| (status, ended_at.is_some(), agent_type))
}

fn ingress_event(
    task_id: &str,
    turn_number: i32,
    event_id: &str,
    sub_type: &str,
    transition: TurnTransition,
) -> TurnStateEvent {
    TurnStateEvent {
        event_id: event_id.to_string(),
        task_id: task_id.to_string(),
        event_type: if sub_type == "active" || sub_type == "turn_interrupted" {
            "taskTurn".to_string()
        } else {
            "agent".to_string()
        },
        sub_type: sub_type.to_string(),
        is_error: sub_type == "error",
        agent_type: Some("claude-code".to_string()),
        turn_number,
        payload: serde_json::json!({}),
        timestamp: "2026-07-12T00:00:00.000Z".to_string(),
        transition,
    }
}

#[sqlx::test]
async fn open_then_terminal_closes_and_is_not_resurrected(pool: PgPool) {
    let repo = PgTaskTurnDao(pool.clone());
    let task = "task-turn-1";

    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e1",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();
    let (status, ended, agent) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "active");
    assert!(!ended, "active turn has no ended_at");
    assert_eq!(agent.as_deref(), Some("claude-code"));

    // Terminal completes the turn and stamps ended_at.
    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e2",
        "completed",
        TurnTransition::Terminal(TurnStatus::Completed),
    ))
    .await
    .unwrap();
    let (status, ended, _) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "completed");
    assert!(ended, "terminal turn stamps ended_at");

    // A re-delivered `active` for the same turn must NOT resurrect a closed turn.
    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e1",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();
    let (status, _, _) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "completed", "terminal turn stays terminal");
}

#[sqlx::test]
async fn terminal_redelivery_preserves_first_status(pool: PgPool) {
    let repo = PgTaskTurnDao(pool.clone());
    let task = "task-turn-2";

    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e1",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();
    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e2",
        "error",
        TurnTransition::Terminal(TurnStatus::Error),
    ))
    .await
    .unwrap();
    // A later stopped terminal for the same closed turn is a no-op.
    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e3",
        "stopped",
        TurnTransition::Terminal(TurnStatus::Stopped),
    ))
    .await
    .unwrap();

    let (status, _, _) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "error", "first terminal status wins");
}

#[sqlx::test]
async fn interrupted_then_resumed(pool: PgPool) {
    let repo = PgTaskTurnDao(pool.clone());
    let task = "task-turn-3";

    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e1",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();
    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e2",
        "turn_interrupted",
        TurnTransition::Interrupted,
    ))
    .await
    .unwrap();
    let (status, _, _) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "interrupted");

    // Resume: an `active` re-opens an interrupted turn.
    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "e3",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();
    let (status, ended, _) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "active");
    assert!(!ended);
}

#[sqlx::test]
async fn terminal_without_open_inserts_row(pool: PgPool) {
    let repo = PgTaskTurnDao(pool.clone());
    let task = "task-turn-4";

    // A lost `active` event: the terminal is the first thing we see.
    repo.apply_ingress_event(&ingress_event(
        task,
        2,
        "e1",
        "completed",
        TurnTransition::Terminal(TurnStatus::Completed),
    ))
    .await
    .unwrap();
    let (status, ended, _) = turn_row(&pool, task, 2).await.unwrap();
    assert_eq!(status, "completed");
    assert!(ended);
}

#[sqlx::test]
async fn ingress_writes_event_row_idempotently(pool: PgPool) {
    let repo = PgTaskTurnDao(pool.clone());
    let task = "task-turn-5";

    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "shared-id",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();
    // Same event_id again (redelivery) — ON CONFLICT DO NOTHING keeps one row.
    repo.apply_ingress_event(&ingress_event(
        task,
        1,
        "shared-id",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_events WHERE event_id = $1")
        .bind("shared-id")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "event row deduped on event_id");
}

#[sqlx::test]
async fn terminal_reopens_task_but_not_when_closed(pool: PgPool) {
    // Seed the FK chain (team → repo → task) so the tasks row exists.
    sqlx::query(
        r#"INSERT INTO teams (id, name, slug) VALUES ($1, $2, $1)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind("personal-user-1")
    .bind("user-1 Personal")
    .execute(&pool)
    .await
    .unwrap();
    moatless_backend::repository::dao::insert(&pool, &make_repository_row("repo-1", "user-1"))
        .await
        .unwrap();

    let repo = PgTaskTurnDao(pool.clone());

    // Task in a non-closed, non-open state → terminal flips it back to Open.
    let open_task = make_task_row("task-reopen", "user-1");
    task_dao::insert_task(&pool, &open_task).await.unwrap();
    sqlx::query("UPDATE tasks SET status = 'error' WHERE task_id = $1")
        .bind("task-reopen")
        .execute(&pool)
        .await
        .unwrap();

    repo.apply_ingress_event(&ingress_event(
        "task-reopen",
        1,
        "e1",
        "completed",
        TurnTransition::Terminal(TurnStatus::Completed),
    ))
    .await
    .unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM tasks WHERE task_id = $1")
        .bind("task-reopen")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status,
        TaskStatus::Open.as_str(),
        "terminal reopens a live task"
    );

    // A Closed task is never resurrected by an automated terminal event.
    let closed_task = make_task_row("task-closed", "user-1");
    task_dao::insert_task(&pool, &closed_task).await.unwrap();
    sqlx::query("UPDATE tasks SET status = 'closed' WHERE task_id = $1")
        .bind("task-closed")
        .execute(&pool)
        .await
        .unwrap();

    repo.apply_ingress_event(&ingress_event(
        "task-closed",
        1,
        "e2",
        "completed",
        TurnTransition::Terminal(TurnStatus::Completed),
    ))
    .await
    .unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM tasks WHERE task_id = $1")
        .bind("task-closed")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "closed", "closed task stays closed");
}

#[sqlx::test]
async fn project_turn_is_idempotent_fallback(pool: PgPool) {
    let repo = PgTaskTurnDao(pool.clone());
    let task = "task-turn-6";
    let now = Utc::now();

    // The consumer fallback touches only task_turns (no event row).
    repo.project_turn(
        task,
        1,
        Some("codex".to_string()),
        TurnTransition::Open,
        now,
    )
    .await
    .unwrap();
    let (status, _, agent) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "active");
    assert_eq!(agent.as_deref(), Some("codex"));

    // Re-applying the same Open is a no-op (row already current).
    repo.project_turn(
        task,
        1,
        Some("codex".to_string()),
        TurnTransition::Open,
        now,
    )
    .await
    .unwrap();
    let (status, _, _) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "active");

    // Terminal via fallback closes it; no task_events row was written.
    repo.project_turn(
        task,
        1,
        Some("codex".to_string()),
        TurnTransition::Terminal(TurnStatus::Completed),
        now,
    )
    .await
    .unwrap();
    let (status, ended, _) = turn_row(&pool, task, 1).await.unwrap();
    assert_eq!(status, "completed");
    assert!(ended);
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_events WHERE task_id = $1")
        .bind(task)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(events, 0, "fallback projection does not write event rows");
}

#[sqlx::test]
async fn list_unclosed_active_turns_filters_by_age_and_status(pool: PgPool) {
    let old = Utc::now() - Duration::hours(1);
    let recent = Utc::now();

    // Directly seed task_turns with controlled started_at / status.
    let seed = |task_id: &'static str,
                turn_number: i32,
                status: &'static str,
                started_at: chrono::DateTime<Utc>| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                r#"INSERT INTO task_turns (task_id, turn_number, status, agent_type, started_at)
                   VALUES ($1, $2, $3, 'claude-code', $4)"#,
            )
            .bind(task_id)
            .bind(turn_number)
            .bind(status)
            .bind(started_at)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    seed("old-active", 1, "active", old).await;
    seed("old-interrupted", 1, "interrupted", old).await;
    seed("old-completed", 1, "completed", old).await;
    seed("recent-active", 1, "active", recent).await;
    seed("stale-active", 1, "active", old).await;
    seed("stale-active", 2, "completed", recent).await;

    let repo = PgTaskTurnDao(pool.clone());
    let cutoff = Utc::now() - Duration::minutes(5);
    let stuck = repo.list_unclosed_active_turns(cutoff).await.unwrap();

    let mut ids: Vec<String> = stuck.into_iter().map(|t| t.task_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["old-active", "old-interrupted"]);
}

#[sqlx::test]
async fn backfill_recovers_open_turns_from_event_log(pool: PgPool) {
    // Simulate the pre-migration event log: task A's latest taskTurn is active,
    // task B's latest is completed (closed). The migration's backfill already ran
    // on an empty table, so we re-run the same statement against seeded events.
    let insert_event = |task_id: &'static str, sub_type: &'static str, turn: i32, secs_ago: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                r#"INSERT INTO task_events
                       (event_id, task_id, type, sub_type, turn_number, agent_type, timestamp, created_at)
                   VALUES ($1, $2, 'taskTurn', $3, $4, 'claude-code', '', now() - ($5 || ' seconds')::interval)"#,
            )
            .bind(format!("{task_id}-{sub_type}-{turn}"))
            .bind(task_id)
            .bind(sub_type)
            .bind(turn)
            .bind(secs_ago.to_string())
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    // Task A: active is the latest event → should backfill.
    insert_event("task-a", "active", 1, 10).await;
    // Task B: active then completed (completed is latest) → should NOT backfill.
    insert_event("task-b", "active", 1, 20).await;
    insert_event("task-b", "completed", 1, 5).await;

    // The exact backfill statement from migration 0071.
    sqlx::query(
        r#"INSERT INTO task_turns (task_id, turn_number, status, agent_type, started_at)
           SELECT latest.task_id, COALESCE(latest.turn_number, 1), 'active',
                  latest.agent_type, latest.created_at
           FROM (
               SELECT DISTINCT ON (task_id)
                      task_id, sub_type, turn_number, agent_type, created_at
               FROM task_events
               WHERE type = 'taskTurn'
               ORDER BY task_id, created_at DESC, id DESC
           ) latest
           WHERE latest.sub_type = 'active'
           ON CONFLICT (task_id, turn_number) DO NOTHING"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        turn_row(&pool, "task-a", 1).await.is_some(),
        "active turn backfilled"
    );
    let (status, _, _) = turn_row(&pool, "task-a", 1).await.unwrap();
    assert_eq!(status, "active");
    assert!(
        turn_row(&pool, "task-b", 1).await.is_none(),
        "closed turn not backfilled"
    );
}

// ── resolve_turn_for_run — the turn allocator ───────────────────────────────

/// Seed a repo + task so `task_turns` / `task_messages` FKs resolve.
async fn seed_task(pool: &PgPool, task_id: &str) {
    let repo = make_repository_row("repo-1", "user-1");
    moatless_backend::repository::dao::insert(pool, &repo)
        .await
        .expect("seed repo");
    let task = make_task_row(task_id, "user-1");
    task_dao::insert_task(pool, &task).await.expect("seed task");
}

#[sqlx::test]
async fn resolve_turn_for_run_is_idempotent(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());

    let first = repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();
    let second = repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();

    assert_eq!(first, 1);
    assert_eq!(second, first, "the same run always maps to the same turn");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_turns WHERE task_id = $1")
        .bind("task-a")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "one run must not create two turns");
}

#[sqlx::test]
async fn resolve_turn_for_run_converges_whichever_signal_lands_first(pool: PgPool) {
    // The ack and `taskTurn.active` travel on unsynchronized channels, so the
    // allocator must not care which of them calls first.
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());

    // Ack first, then the (outbox-delayed) turn-start event.
    let acked = repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();
    let announced = repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();
    assert_eq!(acked, announced);

    // A second run gets its own turn regardless of arrival interleaving.
    let next = repo.resolve_turn_for_run("task-a", "run-2").await.unwrap();
    assert_eq!(next, acked + 1);
}

#[sqlx::test]
async fn resolve_turn_for_run_closes_a_stale_open_turn(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());

    repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();
    repo.resolve_turn_for_run("task-a", "run-2").await.unwrap();

    let (status, _, _) = turn_row(&pool, "task-a", 1).await.unwrap();
    assert_eq!(
        status, "interrupted",
        "a new run leaves at most one open turn behind"
    );
    let (status, _, _) = turn_row(&pool, "task-a", 2).await.unwrap();
    assert_eq!(status, "active");
}

#[sqlx::test]
async fn resolve_turn_for_run_respects_legacy_message_turns(pool: PgPool) {
    // Migration 0071 seeded `task_turns` only for turns active at rollout, so a
    // long-running task can have an empty table and a deep message history.
    seed_task(&pool, "task-a").await;
    let mut msg = common::make_message_row("task-a", "legacy-user");
    msg.turn_number = Some(47);
    moatless_backend::message::dao::append(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let repo = PgTaskTurnDao(pool.clone());
    let allocated = repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();

    assert_eq!(allocated, 48, "allocation must clear the message history");
}

#[sqlx::test]
async fn latest_turn_number_is_none_for_a_task_with_no_turns(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());
    assert_eq!(repo.latest_turn_number("task-a").await.unwrap(), None);

    repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();
    assert_eq!(repo.latest_turn_number("task-a").await.unwrap(), Some(1));
}

#[sqlx::test]
async fn increment_recovery_attempts_counts_per_turn(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());
    repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();

    assert_eq!(
        repo.increment_recovery_attempts("task-a", 1).await.unwrap(),
        1
    );
    assert_eq!(
        repo.increment_recovery_attempts("task-a", 1).await.unwrap(),
        2
    );
}

#[sqlx::test]
async fn reset_recovery_attempts_zeroes_the_counter(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());
    repo.resolve_turn_for_run("task-a", "run-1").await.unwrap();
    repo.increment_recovery_attempts("task-a", 1).await.unwrap();
    repo.increment_recovery_attempts("task-a", 1).await.unwrap();

    repo.reset_recovery_attempts("task-a", 1).await.unwrap();

    // The next attempt starts counting from zero again.
    assert_eq!(
        repo.increment_recovery_attempts("task-a", 1).await.unwrap(),
        1
    );

    // A missing row is a no-op, not an error.
    repo.reset_recovery_attempts("task-a", 999).await.unwrap();
}

// ── latest_closed_turn_status — the resume-suppression signal ───────────────

#[sqlx::test]
async fn latest_closed_turn_status_skips_the_turn_still_running(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());

    assert_eq!(
        repo.latest_closed_turn_status("task-a").await.unwrap(),
        None,
        "a task with no turns has nothing to report"
    );

    // Turn 1 is stopped, turn 2 is the run asking whether it may resume. The
    // open turn must not mask the stop that precedes it.
    repo.apply_ingress_event(&ingress_event(
        "task-a",
        1,
        "e1",
        "stopped",
        TurnTransition::Terminal(TurnStatus::Stopped),
    ))
    .await
    .unwrap();
    repo.apply_ingress_event(&ingress_event(
        "task-a",
        2,
        "e2",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();

    assert_eq!(
        repo.latest_closed_turn_status("task-a").await.unwrap(),
        Some(TurnStatus::Stopped)
    );
}

#[sqlx::test]
async fn latest_closed_turn_status_tracks_the_newest_closed_turn(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());

    repo.apply_ingress_event(&ingress_event(
        "task-a",
        1,
        "e1",
        "stopped",
        TurnTransition::Terminal(TurnStatus::Stopped),
    ))
    .await
    .unwrap();
    repo.apply_ingress_event(&ingress_event(
        "task-a",
        2,
        "e2",
        "completed",
        TurnTransition::Terminal(TurnStatus::Completed),
    ))
    .await
    .unwrap();

    assert_eq!(
        repo.latest_closed_turn_status("task-a").await.unwrap(),
        Some(TurnStatus::Completed),
        "a clean turn after a stop makes the session resumable again"
    );
}

// ── the running rule's two reads ────────────────────────────────────────────

/// Seed a closed turn directly, with the `ended_at` the candidate query compares.
async fn seed_closed_turn(
    pool: &PgPool,
    task_id: &str,
    turn_number: i32,
    status: &str,
    ended_at: Option<chrono::DateTime<Utc>>,
) {
    sqlx::query(
        r#"INSERT INTO task_turns (task_id, turn_number, status, agent_type, started_at, ended_at, run_id)
           VALUES ($1, $2, $3, 'claude-code', now() - interval '1 hour', $4, $5)"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .bind(status)
    .bind(ended_at)
    .bind(format!("{task_id}-run-{turn_number}"))
    .execute(pool)
    .await
    .unwrap();
}

/// Seed one event dated by `created_at`, which is the column the candidate query
/// reads. `timestamp` is deliberately left free-form: nothing may parse it.
async fn seed_event(pool: &PgPool, task_id: &str, sub_type: &str, minutes_ago: i64) {
    sqlx::query(
        r#"INSERT INTO task_events
               (event_id, task_id, type, sub_type, turn_number, agent_type, timestamp, created_at)
           VALUES ($1, $2, 'agent', $3, 1, 'claude-code', 'not-a-timestamp', $4)"#,
    )
    .bind(format!("{task_id}-{sub_type}-{minutes_ago}"))
    .bind(task_id)
    .bind(sub_type)
    .bind(Utc::now() - Duration::minutes(minutes_ago))
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn settled_turns_with_later_activity_are_the_only_candidates(pool: PgPool) {
    let closed = Utc::now() - Duration::minutes(30);

    // The candidate: newest turn closed, and the agent has spoken since.
    seed_closed_turn(&pool, "candidate", 1, "completed", Some(closed)).await;
    seed_event(&pool, "candidate", "activity", 5).await;

    // Closed, but nothing has happened since it closed.
    seed_closed_turn(&pool, "quiet", 1, "completed", Some(closed)).await;
    seed_event(&pool, "quiet", "activity", 45).await;

    // Activity since, but the turn is still open — the orphan arm's business.
    seed_closed_turn(&pool, "still-open", 1, "active", None).await;
    seed_event(&pool, "still-open", "activity", 5).await;

    // Closed with no `ended_at`: nothing to compare against, never a candidate.
    seed_closed_turn(&pool, "no-ended-at", 1, "stopped", None).await;
    seed_event(&pool, "no-ended-at", "activity", 5).await;

    // The newest turn is open, so the closed one below it is not the task's state.
    seed_closed_turn(&pool, "superseded", 1, "completed", Some(closed)).await;
    seed_closed_turn(&pool, "superseded", 2, "active", None).await;
    seed_event(&pool, "superseded", "activity", 5).await;

    // A later event that is not agent activity says nothing about a live run.
    seed_closed_turn(&pool, "wrong-event", 1, "completed", Some(closed)).await;
    seed_event(&pool, "wrong-event", "message", 5).await;

    let repo = PgTaskTurnDao(pool.clone());
    let settled = repo.list_settled_turns_with_later_activity().await.unwrap();

    let mut ids: Vec<String> = settled.iter().map(|t| t.task_id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["candidate"]);
    let found = &settled[0];
    assert_eq!(found.turn_number, 1);
    assert_eq!(found.agent_type.as_deref(), Some("claude-code"));
    assert_eq!(
        found.run_id.as_deref(),
        Some("candidate-run-1"),
        "the closed turn's run is what a heartbeat is compared against"
    );
}

#[sqlx::test]
async fn latest_running_signals_reads_the_newest_turn_and_the_newest_person(pool: PgPool) {
    let repo = PgTaskTurnDao(pool.clone());

    assert_eq!(
        repo.latest_running_signals("nothing-here").await.unwrap(),
        TurnSignals::default(),
        "a task with no turns and no messages is an empty answer, not an error"
    );

    let ended = Utc::now() - Duration::minutes(30);
    seed_closed_turn(&pool, "task-a", 1, "completed", Some(ended)).await;
    seed_closed_turn(
        &pool,
        "task-a",
        2,
        "stopped",
        Some(ended + Duration::minutes(10)),
    )
    .await;

    let insert_message = |uuid: &'static str, message_type: &'static str, minutes_ago: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                r#"INSERT INTO task_messages (task_id, uuid, message_type, user_type, timestamp,
                                              message_role, message_content, created_at)
                   VALUES ('task-a', $1, $2, 'human', '', $2, '{}'::jsonb, $3)"#,
            )
            .bind(uuid)
            .bind(message_type)
            .bind(Utc::now() - Duration::minutes(minutes_ago))
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    insert_message("m1", "user", 40).await;
    insert_message("m2", "user", 20).await;
    // An assistant message is newer than either, and must not be mistaken for
    // the person's — a reply is not a new request.
    insert_message("m3", "assistant", 1).await;

    let signals = repo.latest_running_signals("task-a").await.unwrap();
    assert_eq!(
        signals.turn_status,
        Some(TurnStatus::Stopped),
        "the newest turn by number is the task's state"
    );
    let turn_ended = signals.turn_ended_at.expect("closed turn stamps ended_at");
    assert!(
        (turn_ended - (ended + Duration::minutes(10)))
            .num_seconds()
            .abs()
            < 2
    );
    let asked = signals
        .latest_user_message_at
        .expect("a task with user messages reports one");
    assert!(
        (asked - (Utc::now() - Duration::minutes(20)))
            .num_seconds()
            .abs()
            < 5,
        "the newest user message, not the newest message"
    );
}
/// A turn left `interrupted` behind later completed turns is a lost terminal
/// event, not work in flight. Task 062dc4f6 sat like this: turn 1 interrupted,
/// turns 2 through 8 completed, sandbox stopped. Reading the lowest open turn
/// reported turn 1 as in flight, which nothing could ever clear.
#[sqlx::test]
async fn a_stale_open_turn_behind_a_finished_one_does_not_block_a_fork(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());

    repo.apply_ingress_event(&ingress_event(
        "task-a",
        1,
        "e1-active",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();
    repo.apply_ingress_event(&ingress_event(
        "task-a",
        1,
        "e1-interrupted",
        "turn_interrupted",
        TurnTransition::Interrupted,
    ))
    .await
    .unwrap();
    repo.apply_ingress_event(&ingress_event(
        "task-a",
        2,
        "e2-completed",
        "completed",
        TurnTransition::Terminal(TurnStatus::Completed),
    ))
    .await
    .unwrap();

    let point = repo.fork_point("task-a", None).await.unwrap();

    assert_eq!(
        point.open_turn, None,
        "turn 1 cannot still be running once turn 2 has finished"
    );
    assert_eq!(
        point.turn,
        Some(2),
        "the cut lands on the newest ended turn"
    );
}

/// The newest turn still being open is what a fork must refuse, and that check
/// has to keep working now that it reads one row instead of aggregating.
#[sqlx::test]
async fn a_running_newest_turn_still_blocks_a_fork(pool: PgPool) {
    seed_task(&pool, "task-a").await;
    let repo = PgTaskTurnDao(pool.clone());

    repo.apply_ingress_event(&ingress_event(
        "task-a",
        1,
        "e1-completed",
        "completed",
        TurnTransition::Terminal(TurnStatus::Completed),
    ))
    .await
    .unwrap();
    repo.apply_ingress_event(&ingress_event(
        "task-a",
        2,
        "e2-active",
        "active",
        TurnTransition::Open,
    ))
    .await
    .unwrap();

    let point = repo.fork_point("task-a", None).await.unwrap();

    assert_eq!(point.open_turn, Some(2));
}

/// The reaper's second turn pass. `list_unclosed_active_turns` returns only a
/// task's newest open turn, so a turn a later turn ran past is invisible to it;
/// this query is what finds those, keyed on being open and below the task's
/// high-water mark.
#[sqlx::test]
async fn list_superseded_open_turns_returns_only_turns_a_later_turn_ran_past(pool: PgPool) {
    let old = Utc::now() - Duration::hours(1);
    let recent = Utc::now();

    let seed = |task_id: &'static str,
                turn_number: i32,
                status: &'static str,
                started_at: chrono::DateTime<Utc>| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                r#"INSERT INTO task_turns (task_id, turn_number, status, agent_type, started_at)
                   VALUES ($1, $2, $3, 'claude-code', $4)"#,
            )
            .bind(task_id)
            .bind(turn_number)
            .bind(status)
            .bind(started_at)
            .execute(&pool)
            .await
            .unwrap();
        }
    };

    // Superseded: turn 1 still open, turn 2 already finished.
    seed("stranded", 1, "interrupted", old).await;
    seed("stranded", 2, "completed", recent).await;
    // The newest turn is the open one — the other pass owns this, not us.
    seed("newest-open", 1, "completed", old).await;
    seed("newest-open", 2, "active", old).await;
    // Open and superseded but not yet past the age floor.
    seed("too-recent", 1, "active", recent).await;
    seed("too-recent", 2, "completed", recent).await;
    // Superseded but already terminal — nothing to settle.
    seed("already-done", 1, "completed", old).await;
    seed("already-done", 2, "completed", recent).await;

    let repo = PgTaskTurnDao(pool.clone());
    let cutoff = Utc::now() - Duration::minutes(5);
    let superseded = repo.list_superseded_open_turns(cutoff).await.unwrap();

    let ids: Vec<(String, Option<i32>)> = superseded
        .into_iter()
        .map(|t| (t.task_id, t.turn_number))
        .collect();
    assert_eq!(ids, vec![("stranded".to_string(), Some(1))]);
}

/// Settling a superseded turn moves its row to terminal and takes it off the
/// list, without disturbing the newest turn that superseded it.
#[sqlx::test]
async fn a_superseded_turn_settles_to_terminal_and_leaves_the_newest_alone(pool: PgPool) {
    let old = Utc::now() - Duration::hours(1);
    let repo = PgTaskTurnDao(pool.clone());

    sqlx::query(
        r#"INSERT INTO task_turns (task_id, turn_number, status, agent_type, started_at)
           VALUES ('task-a', 1, 'interrupted', 'claude-code', $1),
                  ('task-a', 2, 'completed',   'claude-code', $1)"#,
    )
    .bind(old)
    .execute(&pool)
    .await
    .unwrap();

    let cutoff = Utc::now() - Duration::minutes(5);
    assert_eq!(
        repo.list_superseded_open_turns(cutoff).await.unwrap().len(),
        1
    );

    repo.project_turn(
        "task-a",
        1,
        Some("claude-code".to_string()),
        TurnTransition::Terminal(TurnStatus::Stopped),
        Utc::now(),
    )
    .await
    .unwrap();

    assert_eq!(
        turn_row(&pool, "task-a", 1)
            .await
            .map(|(s, ended, _)| (s, ended)),
        Some(("stopped".to_string(), true)),
        "the stranded turn is now terminal"
    );
    assert_eq!(
        turn_row(&pool, "task-a", 2).await.map(|(s, _, _)| s),
        Some("completed".to_string()),
        "the newest turn is untouched"
    );
    assert!(
        repo.list_superseded_open_turns(cutoff)
            .await
            .unwrap()
            .is_empty(),
        "a settled turn is no longer listed"
    );
}
