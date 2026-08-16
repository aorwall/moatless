//! Shared seeding for the `db-tests` lifecycle modules.
//!
//! One helper so the store, lock, and contract tests share a single task-row
//! shape instead of three drifting copies.

use sqlx::PgPool;

// Re-exported at this path so the lifecycle db-tests keep reaching the seeder as
// `super::super::test_support::seed_bare_task` — the tasks band now owns it.
pub(crate) use moatless_tasks::task::test_support::seed_bare_task;

/// Seed a team and one task in the default `open` / `running` shape.
pub(crate) async fn seed_task(pool: &PgPool, task_id: &str) {
    seed_task_with(pool, task_id, "open", "running").await;
}

/// Seed a team, one task, and its Sandbox row with an explicit status and
/// desired state. Production is born lazily through lifecycle; the fixture
/// records the row directly to mirror a Task that has already declared intent.
///
/// The bare task/team rows come from the tasks band's seeder (`moatless_tasks`
/// owns those tables); this layers the sandbox-control band's `sandboxes` row.
pub(crate) async fn seed_task_with(pool: &PgPool, task_id: &str, status: &str, desired: &str) {
    seed_bare_task(pool, task_id, status).await;
    sqlx::query(
        r#"INSERT INTO sandboxes (sandbox_id, workspace_id, desired_state, desired_state_updated_at)
           SELECT $1, COALESCE(workspace_id, 'ws_' || repository_id), $2, NOW()
           FROM tasks WHERE task_id = $1"#,
    )
    .bind(task_id)
    .bind(desired)
    .execute(pool)
    .await
    .expect("seed sandbox");
    sqlx::query("UPDATE tasks SET sandbox_id = $1 WHERE task_id = $1")
        .bind(task_id)
        .execute(pool)
        .await
        .expect("bind task to sandbox");
}
