// Compiles only for test builds or `feature = "test-support"`, never a
// shipping build, so the seeder's `.expect()` calls are test scaffolding —
// allowed like the crate's own `#[cfg(test)]` code (the crate-level allow is
// `test`-gated and misses the feature-gated build).
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Shared `db-tests` seeding for the tasks band.
//!
//! Gated `#[cfg(any(test, feature = "test-support"))]`, so it compiles for this
//! crate's own db-tests and, off `test-support`, for the sandbox-control band's
//! lifecycle db-tests (still in the backend), which seed the same task row
//! shape before layering their own `sandboxes` row on top. The tasks band owns
//! the `tasks`/`teams` tables, so it owns their seeder.

use sqlx::PgPool;

/// Seed a team, one task, and the Sandbox row task creation writes for it — the
/// shape a Task has before any lifecycle operation: converged Stopped at the
/// intent epoch, owning no volume.
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
        r#"INSERT INTO tasks (task_id, created_at, last_message_at, agent_profile, repository_id,
               status, total_cost_usd, visibility, scope, team_id, created_by, owner_user_id,
               workspace_id, sandbox_id)
           VALUES ($1, to_char(NOW() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"'),
               to_char(NOW() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"'),
               'claude-code', 'repo-x', $2, 0,
               'private', 'user', 'team-x', 'user-1', 'user-1', 'ws_repo-x', $1)"#,
    )
    .bind(task_id)
    .bind(status)
    .execute(pool)
    .await
    .expect("seed task");
}

// =============================================================================
// Named fixtures over the tasks band's stores
// =============================================================================
//
// A test that needs a Task, a relation or a listing reaches the store through
// one of these rather than through the store itself.

use moatless_core::db::DbPool;
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentProfile;

use super::dao::{PgTaskDao, TaskDao, store_over};
use super::models::{Task, TaskFilter, TaskPatch, TaskStatus, Visibility};
use super::relations::TaskRelation;
use super::relations_dao::{PgTaskRelationDao, TaskRelationDao};

/// The store over a pool, with the shadow-workspace seam wired to the same
/// pool. Every fixture below builds on it.
fn tasks(pool: &DbPool) -> PgTaskDao {
    store_over(pool)
}

/// Insert a Task, creating `row.workspace_id` first if absent. A fixture rarely
/// seeds its own Workspace, and [`TaskDao::insert_task`] refuses an absent one;
/// [`store_over`] tests that refusal.
pub async fn seed_task(pool: &DbPool, row: &Task) -> Result<(), sqlx::Error> {
    if let Some(workspace_id) = row.workspace_id.as_deref() {
        sqlx::query(
            "INSERT INTO workspaces (id, name, scope, owner_user_id) \
             VALUES ($1, $1, 'user', $2) ON CONFLICT (id) DO NOTHING",
        )
        .bind(workspace_id)
        .bind(row.owner_user_id.as_deref().or(row.created_by.as_deref()))
        .execute(pool)
        .await?;
    }
    tasks(pool)
        .insert_task(row, &crate::task::models::SandboxPlacement::default())
        .await
}

/// Read one Task back, scoped to `ctx`.
pub async fn find_task(
    pool: &DbPool,
    task_id: &str,
    ctx: &ScopeContext,
) -> Result<Option<Task>, sqlx::Error> {
    tasks(pool).find_by_id(task_id, ctx).await
}

/// Apply a partial update to a Task, scoped to `ctx`.
pub async fn patch_task(
    pool: &DbPool,
    task_id: &str,
    patch: &TaskPatch,
    ctx: &ScopeContext,
) -> Result<bool, sqlx::Error> {
    tasks(pool).update_task(task_id, patch, ctx).await
}

/// Flip a Task's status unless it is already `Closed`, scoped to `ctx`.
pub async fn set_status_if_open(
    pool: &DbPool,
    task_id: &str,
    new_status: TaskStatus,
    ctx: &ScopeContext,
) -> Result<bool, sqlx::Error> {
    tasks(pool)
        .update_status_if_not_closed(task_id, new_status, ctx)
        .await
}

/// Delete a Task, scoped to `ctx`.
pub async fn remove_task(
    pool: &DbPool,
    task_id: &str,
    ctx: &ScopeContext,
) -> Result<bool, sqlx::Error> {
    tasks(pool).delete_task(task_id, ctx).await
}

/// Page the Task listing.
pub async fn list_tasks(
    pool: &DbPool,
    filter: &TaskFilter,
) -> Result<(Vec<Task>, i64), sqlx::Error> {
    tasks(pool).list_tasks(filter).await
}

/// Recompute a Task's total cost from its turn metrics.
pub async fn recompute_total_cost(pool: &DbPool, task_id: &str) -> Result<(), sqlx::Error> {
    tasks(pool).recompute_total_cost(task_id).await
}

/// Whether a Task carries this `short_id`.
pub async fn short_id_exists(pool: &DbPool, short_id: &str) -> Result<bool, sqlx::Error> {
    tasks(pool).exists_by_short_id(short_id).await
}

/// Make `user_id` follow `task_id`.
pub async fn follow_task(pool: &DbPool, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
    tasks(pool).follow_task(user_id, task_id).await
}

/// Make `user_id` stop following `task_id`.
pub async fn unfollow_task(pool: &DbPool, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
    tasks(pool).unfollow_task(user_id, task_id).await
}

/// Settle `task_id` for `user_id` at `at`.
pub async fn settle_task(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    tasks(pool).settle_task(user_id, task_id, at).await
}

/// Snooze `task_id` for `user_id` until `until`, stamped at `at`.
pub async fn snooze_task(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
    until: chrono::DateTime<chrono::Utc>,
    at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    tasks(pool).snooze_task(user_id, task_id, until, at).await
}

/// Unsettle `task_id` for `user_id`.
pub async fn unsettle_task(pool: &DbPool, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
    tasks(pool).unsettle_task(user_id, task_id).await
}

/// Insert a typed relation between two Tasks.
pub async fn seed_relation(pool: &DbPool, row: &TaskRelation) -> Result<(), sqlx::Error> {
    PgTaskRelationDao(pool.clone()).insert(row).await
}

// =============================================================================
// Row factories
// =============================================================================

/// The Workspace [`make_task_row`] runs its task in. Seed the row before
/// inserting: [`seed_task`] refuses a Task whose Workspace does not exist.
pub const FIXTURE_WORKSPACE: &str = "ws-1";

/// A minimal user-scoped Task, open and privately visible.
pub fn make_task_row(task_id: &str, user_id: &str) -> Task {
    let now = chrono::Utc::now();
    Task {
        task_id: task_id.to_string(),
        short_id: Some(format!("s-{task_id}")),
        created_at: now,
        last_message_at: now,
        agent_profile: AgentProfile::ClaudeCode,
        repository_id: Some("repo-1".to_string()),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: None,
        vibe: None,
        name: Some(format!("Test task {task_id}")),
        description: None,
        site_id: None,
        site_provider: None,
        branch: Some("main".to_string()),
        model: None,
        created_by: Some(user_id.to_string()),
        reporter_user_id: None,
        visibility: Visibility::Private,
        scope: moatless_types::access_control::Scope::User,
        owner_user_id: Some(user_id.to_string()),
        run_as_user_id: None,
        updated_at: None,
        append_system_prompt: None,
        headless: false,
        ttl_minutes: 15,
        close_after_settled_days: 7,
        settle_after_hours: None,
        loop_id: None,
        workspace_id: Some(FIXTURE_WORKSPACE.to_string()),
        sandbox_id: None,
        checkout_mode: None,
        memory_preload: Vec::new(),
        agent_properties: Default::default(),
        skills: Vec::new(),
    }
}

// =============================================================================
// A scripted store, for a host test with no database
// =============================================================================

use std::sync::{Arc, Mutex};

use super::dao::MockTaskDao;
use super::operations::TaskCommands;

/// What a path asked the store, in call order.
#[derive(Default)]
pub struct TaskStoreCalls {
    find_by_id: Mutex<Vec<(String, ScopeContext)>>,
    list_tasks: Mutex<Vec<TaskFilter>>,
    delete_errored_empty_task: Mutex<Vec<(String, ScopeContext)>>,
}

impl TaskStoreCalls {
    /// The `(task_id, scope)` pairs a path read a Task by.
    pub fn find_by_id(&self) -> Vec<(String, ScopeContext)> {
        self.find_by_id.lock().unwrap().clone()
    }

    /// The filters a path paged the listing with.
    pub fn list_tasks(&self) -> Vec<TaskFilter> {
        self.list_tasks.lock().unwrap().clone()
    }

    /// The `(task_id, scope)` pairs a path tried to free an errored id under.
    pub fn delete_errored_empty_task(&self) -> Vec<(String, ScopeContext)> {
        self.delete_errored_empty_task.lock().unwrap().clone()
    }
}

/// A task store scripted for a host test: answers gate-path reads and records
/// what was asked, so a test asserts scope, filter and count with no database.
/// Unscripted reads answer "nothing there".
#[derive(Default)]
pub struct ScriptedTasks {
    holding: Option<Task>,
    listing: (Vec<Task>, i64),
    frees_errored_ids: bool,
}

impl ScriptedTasks {
    pub fn new() -> Self {
        Self::default()
    }

    /// The Task a scoped read answers with. `None` models a Task that is
    /// missing or invisible to the caller.
    pub fn holding(mut self, task: Option<Task>) -> Self {
        self.holding = task;
        self
    }

    /// The page and the unpaged total the listing answers with.
    pub fn listing(mut self, rows: Vec<Task>, total: i64) -> Self {
        self.listing = (rows, total);
        self
    }

    /// Whether freeing an errored empty Task's id reports a row removed.
    pub fn frees_errored_ids(mut self, freed: bool) -> Self {
        self.frees_errored_ids = freed;
        self
    }

    /// The scripted store behind the task command handle.
    pub fn commands(self) -> (Arc<TaskCommands>, Arc<TaskStoreCalls>) {
        let calls = Arc::new(TaskStoreCalls::default());
        (
            Arc::new(TaskCommands::new(Arc::new(self.store(&calls)))),
            calls,
        )
    }

    fn store(self, calls: &Arc<TaskStoreCalls>) -> MockTaskDao {
        let mut store = MockTaskDao::new();

        let holding = self.holding;
        let recorded = Arc::clone(calls);
        store.expect_find_by_id().returning(move |task_id, ctx| {
            recorded
                .find_by_id
                .lock()
                .unwrap()
                .push((task_id.to_string(), ctx.clone()));
            Ok(holding.clone())
        });

        let listing = self.listing;
        let recorded = Arc::clone(calls);
        store.expect_list_tasks().returning(move |filter| {
            recorded.list_tasks.lock().unwrap().push(filter.clone());
            Ok(listing.clone())
        });

        let freed = self.frees_errored_ids;
        let recorded = Arc::clone(calls);
        store
            .expect_delete_errored_empty_task()
            .returning(move |task_id, ctx| {
                recorded
                    .delete_errored_empty_task
                    .lock()
                    .unwrap()
                    .push((task_id.to_string(), ctx.clone()));
                Ok(freed)
            });

        store
    }
}
