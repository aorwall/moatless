//! Named `db-tests` fixtures over the `task_turns` table.
//!
//! A test that needs the gRPC ingress writer's transaction, or the turn a run
//! allocates, reaches the store through one of these rather than through the
//! store itself.

use moatless_core::db::DbPool;

use super::dao::{PgTaskTurnDao, TaskTurnDao};
use super::models::TurnStateEvent;

/// See [`TaskTurnDao::apply_ingress_event`].
pub async fn apply_ingress_event(pool: &DbPool, event: &TurnStateEvent) -> Result<(), sqlx::Error> {
    PgTaskTurnDao(pool.clone()).apply_ingress_event(event).await
}

/// See [`TaskTurnDao::resolve_turn_for_run`].
pub async fn resolve_turn_for_run(
    pool: &DbPool,
    task_id: &str,
    run_id: &str,
) -> Result<i32, sqlx::Error> {
    PgTaskTurnDao(pool.clone())
        .resolve_turn_for_run(task_id, run_id)
        .await
}
