//! Shared seeding for the `db-tests` lifecycle modules.
//!
//! One helper so the store, lock, and contract tests share a single task-row
//! shape instead of three drifting copies.

use sqlx::PgPool;

/// Seed a team and one task in the default `open` / `running` shape.
pub(crate) async fn seed_task(pool: &PgPool, task_id: &str) {
    seed_task_with(pool, task_id, "open", "running").await;
}

/// Seed a team and one task with an explicit status and desired sandbox state.
pub(crate) async fn seed_task_with(pool: &PgPool, task_id: &str, status: &str, desired: &str) {
    // tasks.team_id is NOT NULL (no FK); seed a row so the fixture matches the
    // production shape.
    sqlx::query(
        "INSERT INTO teams (id, name, slug) VALUES ('team-x', 'Team X', 'team-x')
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed team");

    sqlx::query(
        r#"INSERT INTO tasks (task_id, created_at, last_message_at, agent_type, repository_id,
               status, total_cost_usd, visibility, scope, team_id, created_by, owner_user_id,
               sandbox_desired_state)
           VALUES ($1, NOW()::text, NOW()::text, 'claude-code', 'repo-x', $2, 0,
               'private', 'user', 'team-x', 'user-1', 'user-1', $3)"#,
    )
    .bind(task_id)
    .bind(status)
    .bind(desired)
    .execute(pool)
    .await
    .expect("seed task");
}
