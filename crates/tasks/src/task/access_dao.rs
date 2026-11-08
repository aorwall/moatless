//! The scoped single-Task reads: one row, or one column of it, under an
//! access rule from `moatless_core::db::access_control`.

use sqlx::AssertSqlSafe;

use super::dao::{PgTaskDao, TaskDao};
use super::models::{Task, TaskRow, TaskStatus};
use moatless_types::access_control::{ScopeContext, ScopedEntity};

/// The E-(b) session-gate port over `TaskDao`: `verify_handler` (identity)
/// holds `Arc<dyn TaskAccess>`, not a `TaskDao`, so identity keeps no `task`
/// import.
#[async_trait::async_trait]
impl moatless_core::http_auth::TaskAccess for PgTaskDao {
    async fn may_access_task(&self, task_id: &str, ctx: &ScopeContext) -> bool {
        // `find_by_id` is already scope-filtered, so a task `ctx` cannot see
        // returns `None`. Fail-closed on a DB error, matching the handler's prior
        // `map_err(|_| Forbidden)`.
        matches!(self.find_by_id(task_id, ctx).await, Ok(Some(_)))
    }
}

impl PgTaskDao {
    /// One Task under whichever rule `grant` states. `grant` must start at
    /// `$2`, since `task_id` takes `$1`; any other start binds the wrong
    /// positions.
    pub(super) async fn find_under(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
        grant: &moatless_core::db::access_control::AccessFilter,
    ) -> Result<Option<Task>, sqlx::Error> {
        // `SELECT *` changes cached result metadata on additive migrations,
        // breaking surviving replicas with `cached plan must not change result type`.
        let sql = format!(
            "SELECT task_id, short_id, created_at, last_message_at, agent_profile,
                    repository_id, workspace_id, sandbox_id, checkout_mode, status,
                    total_cost_usd, site_url, template_id, vibe, name, description,
                    site_id, site_provider, branch, model, created_by, reporter_user_id,
                    visibility, scope, owner_user_id, run_as_user_id, created_at_db,
                    updated_at, append_system_prompt, headless, ttl_minutes,
                    close_after_settled_days, settle_after_hours, loop_id, skills,
                    memory_preload, agent_properties
             FROM tasks WHERE task_id = $1 AND {}",
            grant.sql()
        );
        let row: Option<TaskRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(Task::try_from).transpose()
    }

    /// The status of the Task [`TaskDao::find_by_id`] would return.
    pub(super) async fn status_under(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskStatus>, sqlx::Error> {
        #[derive(sqlx::FromRow)]
        struct StatusRow {
            #[sqlx(try_from = "String")]
            status: TaskStatus,
        }
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "", 2);
        let sql = format!(
            "SELECT status FROM tasks WHERE task_id = $1 AND {}",
            access.sql()
        );
        let row: Option<StatusRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| row.status))
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "access_dao_db_tests.rs"]
mod access_dao_db_tests;
