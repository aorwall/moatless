//! Database tests for the turn a fork's copy closes.

use super::*;
use crate::message::dao::{MessageDao, PgMessageDao};
use crate::task::test_support::{make_task_row, seed_task};
use crate::task_activities::mapping::ConversationWrite;
use crate::task_turn::dao::{PgTaskTurnDao, TaskTurnDao};
use moatless_types::running::{RunningInputs, Working, derive_running};
use moatless_types::sandbox::SandboxDesiredState;
use moatless_workspaces::test_support::make_repository_row;
use sqlx::PgPool;

/// A parent Task whose turn 1 the agent answered, left in `status`; `None`
/// seeds no `task_turns` row, as for a Task older than migration 0071.
async fn seed_parent(pool: &PgPool, status: Option<&str>) {
    let repo = make_repository_row("repo-1", "user-1");
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    seed_task(pool, &make_task_row("parent", "user-1"))
        .await
        .expect("seed task");

    let mut msg = crate::message::test_support::make_message_row("parent", "parent-user");
    msg.agent_session_id = Some("parent-session".to_string());
    msg.turn_number = Some(1);
    msg.sent_at = Some(chrono::Utc::now());
    crate::message::test_support::seed_message(pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let Some(status) = status else { return };
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status, started_at, ended_at)
         VALUES ('parent', 1, $1, NOW() - INTERVAL '2 hours', NOW() - INTERVAL '1 hour')",
    )
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

async fn working(pool: &PgPool, task_id: &str) -> Working {
    let signals = PgTaskTurnDao(pool.clone())
        .latest_running_signals(task_id)
        .await
        .unwrap();
    derive_running(&RunningInputs {
        desired_state: Some(SandboxDesiredState::Running),
        sandbox_up: Some(true),
        turn_status: signals.turn_status,
        turn_ended_at: signals.turn_ended_at,
        latest_user_message_at: signals.latest_user_message_at,
        ..RunningInputs::default()
    })
}

async fn fork_turn_status(pool: &PgPool) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM task_turns WHERE task_id = 'fork'")
        .fetch_optional(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn a_forked_turn_is_neither_owed_nor_interrupted(pool: PgPool) {
    seed_parent(&pool, Some("completed")).await;

    copy_conversation(&pool, "parent", "fork", Some(1))
        .await
        .unwrap();

    assert_eq!(working(&pool, "fork").await, Working::Idle);
    assert_eq!(fork_turn_status(&pool).await.as_deref(), Some("completed"));
    let interrupted = PgMessageDao(pool.clone())
        .find_interrupted_turn("fork")
        .await
        .unwrap();
    assert!(interrupted.is_none());
}

/// A cut turn left `active` by a lost terminal event is not in flight in the
/// fork: nothing runs there to close it.
#[sqlx::test]
async fn a_forked_turn_left_open_in_the_parent_is_copied_stopped(pool: PgPool) {
    seed_parent(&pool, Some("active")).await;

    copy_conversation(&pool, "parent", "fork", Some(1))
        .await
        .unwrap();

    assert_eq!(fork_turn_status(&pool).await.as_deref(), Some("stopped"));
    assert_eq!(working(&pool, "fork").await, Working::Idle);
}

#[sqlx::test]
async fn a_fork_of_a_task_with_no_turn_rows_closes_its_copied_turn(pool: PgPool) {
    seed_parent(&pool, None).await;

    copy_conversation(&pool, "parent", "fork", None)
        .await
        .unwrap();

    assert_eq!(fork_turn_status(&pool).await.as_deref(), Some("completed"));
    assert_eq!(working(&pool, "fork").await, Working::Idle);
    let interrupted = PgMessageDao(pool.clone())
        .find_interrupted_turn("fork")
        .await
        .unwrap();
    assert!(interrupted.is_none());
}

#[sqlx::test]
async fn discarding_a_fork_removes_its_copied_turn(pool: PgPool) {
    seed_parent(&pool, Some("completed")).await;
    copy_conversation(&pool, "parent", "fork", Some(1))
        .await
        .unwrap();

    discard_conversation(&pool, "fork").await.unwrap();

    assert_eq!(fork_turn_status(&pool).await, None);
}
