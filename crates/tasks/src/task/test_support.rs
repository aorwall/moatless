// This module compiles only for test builds (`#[cfg(test)]`) or downstream test
// tiers (`feature = "test-support"`), never a shipping build, so the seeder's
// `.expect()` calls are test scaffolding — allow them the way the crate's own
// `#[cfg(test)]` code is allowed (the crate-level allow is `test`-gated and does
// not cover the feature-gated build).
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Shared `db-tests` seeding for the tasks band.
//!
//! Gated `#[cfg(any(test, feature = "test-support"))]`, so it compiles for this
//! crate's own db-tests and, off `test-support`, for the sandbox-control band's
//! lifecycle db-tests (still in the backend), which seed the same task row shape
//! before layering their own `sandboxes` row on top. The tasks band owns the
//! `tasks`/`teams` tables, so it owns their seeder.

use sqlx::PgPool;

/// Seed a team, one task, and the Sandbox row task creation writes for it —
/// the shape a Task has before any lifecycle operation: converged Stopped at
/// the intent epoch, owning no volume.
pub async fn seed_bare_task(pool: &PgPool, task_id: &str, status: &str) {
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
        "INSERT INTO workspaces (id, name, scope, team_id, owner_user_id, created_by)
         VALUES ('ws_repo-x', 'Repo X', 'user', 'team-x', 'user-1', 'user-1')
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed workspace");

    sqlx::query(
        r#"INSERT INTO sandboxes (sandbox_id, workspace_id, scope, owner_user_id,
               run_as_user_id, storage_reclaimed_at)
           VALUES ($1, 'ws_repo-x', 'user', 'user-1', 'user-1', NOW())"#,
    )
    .bind(task_id)
    .execute(pool)
    .await
    .expect("seed sandbox");

    sqlx::query(
        r#"INSERT INTO tasks (task_id, created_at, last_message_at, agent_type, repository_id,
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
