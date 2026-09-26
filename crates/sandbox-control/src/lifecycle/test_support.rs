// This module compiles only for test builds, so the seeder's `.expect()` calls
// are test scaffolding.
#![allow(clippy::expect_used)]

//! Shared seeding for the `db-tests` lifecycle modules.
//!
//! One helper so the store, lock, and contract tests share a single row shape
//! instead of three drifting copies.
//!
//! Production rows come from
//! `moatless_tasks::task::operations::TaskCommands::insert_task`, whose own
//! db-tests assert that shape; this fixture owes only enough of it for the
//! lifecycle SQL here to run: a Sandbox row, a Task naming it, and its
//! workspace.

use sqlx::PgPool;

/// Seed one Task and the Sandbox row task creation writes for it — the shape
/// before any lifecycle operation: converged Stopped at the intent epoch,
/// owning no volume.
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

    // `sandboxes_workspace_id_fkey` is ON DELETE RESTRICT, so the workspace
    // exists before the Sandbox that names it.
    sqlx::query(
        "INSERT INTO workspaces (id, name, scope, team_id, owner_user_id, created_by)
         VALUES ('ws_repo-x', 'Repo X', 'user', 'team-x', 'user-1', 'user-1')
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed workspace");

    sqlx::query(
        r#"INSERT INTO sandboxes (sandbox_id, workspace_id, scope, owner_user_id,
               run_as_user_id, agent_profile, storage_reclaimed_at)
           VALUES ($1, 'ws_repo-x', 'user', 'user-1', 'user-1', 'claude-code', NOW())"#,
    )
    .bind(task_id)
    .execute(pool)
    .await
    .expect("seed sandbox");

    sqlx::query(
        r#"INSERT INTO tasks (task_id, created_at, last_message_at, agent_profile, repository_id,
               status, total_cost_usd, visibility, scope, team_id, created_by, owner_user_id,
               workspace_id, sandbox_id)
           VALUES ($1, NOW()::text, NOW()::text, 'claude-code', 'repo-x', $2, 0,
               'private', 'user', 'team-x', 'user-1', 'user-1', 'ws_repo-x', $1)"#,
    )
    .bind(task_id)
    .bind(status)
    .execute(pool)
    .await
    .expect("seed task");
}

/// Seed a Task in the default `open` / `running` shape.
pub(crate) async fn seed_task(pool: &PgPool, task_id: &str) {
    seed_task_with(pool, task_id, "open", "running").await;
}

/// Seed a Task, its Sandbox row, and an explicit status and desired state — a
/// Task that has already declared intent.
pub(crate) async fn seed_task_with(pool: &PgPool, task_id: &str, status: &str, desired: &str) {
    seed_bare_task(pool, task_id, status).await;
    sqlx::query(
        r#"UPDATE sandboxes
              SET desired_state = $2, desired_state_updated_at = NOW()
            WHERE sandbox_id = $1"#,
    )
    .bind(task_id)
    .bind(desired)
    .execute(pool)
    .await
    .expect("seed sandbox intent");
}
