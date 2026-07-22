//! Shared seeding for the `db-tests` lifecycle modules.
//!
//! One helper so the store, lock, and contract tests share a single task-row
//! shape instead of three drifting copies.

use sqlx::PgPool;

/// Seed a team and one task in the default `open` / `running` shape.
pub(crate) async fn seed_task(pool: &PgPool, task_id: &str) {
    seed_task_with(pool, task_id, "open", "running").await;
}

/// Seed a team and one task with **no Sandbox row** — the lazy-absence shape a
/// Task has before any lifecycle operation.
pub(crate) async fn seed_bare_task(pool: &PgPool, task_id: &str, status: &str) {
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
               status, total_cost_usd, visibility, scope, team_id, created_by, owner_user_id)
           VALUES ($1, NOW()::text, NOW()::text, 'claude-code', 'repo-x', $2, 0,
               'private', 'user', 'team-x', 'user-1', 'user-1')"#,
    )
    .bind(task_id)
    .bind(status)
    .execute(pool)
    .await
    .expect("seed task");
}

/// Seed a team, one task, and its Sandbox row with an explicit status and
/// desired state. Production is born lazily through lifecycle; the fixture
/// records the row directly to mirror a Task that has already declared intent.
pub(crate) async fn seed_task_with(pool: &PgPool, task_id: &str, status: &str, desired: &str) {
    seed_bare_task(pool, task_id, status).await;
    sqlx::query(
        r#"INSERT INTO sandboxes (task_id, desired_state, desired_state_updated_at)
           VALUES ($1, $2, NOW())"#,
    )
    .bind(task_id)
    .bind(desired)
    .execute(pool)
    .await
    .expect("seed sandbox");
}
