//! Task DAO — queries for task data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use super::models::{SandboxIntentView, Task, TaskStatus, Visibility};
use crate::db::DbPool;
use crate::sandbox::lifecycle::dao::load_sandbox_intent_views;
use crate::shared_models::AgentType;
use moatless_backend_core::scope::{Scope, ScopeContext, ScopedEntity};

/// Filter struct for task listing queries.
pub struct TaskFilter {
    pub scope: ScopeContext,
    pub limit: i64,
    pub offset: i64,
    pub status: Option<TaskStatus>,
    /// Exclude tasks with this status (NULL = no exclusion). Complements `status`.
    pub exclude_status: Option<TaskStatus>,
    pub repository_id: Option<String>,
    pub created_by: Option<String>,
    pub tag: Option<String>,
    /// When set, restrict results to tasks that have a `created_by` relation
    /// pointing at this task id (i.e. children of this parent).
    pub parent_task_id: Option<String>,
    /// When set, restrict results to tasks this user follows (a row exists in
    /// `task_follows`). Drives the personal sidebar list.
    pub followed_by: Option<String>,
}

/// Repository trait for task persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Find by task_id, scoped to access control.
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error>;
    async fn insert_task(&self, row: &Task) -> Result<(), sqlx::Error>;
    async fn list_tasks(&self, filter: &TaskFilter) -> Result<(Vec<Task>, i64), sqlx::Error>;
    /// Update task fields for the caller's scope. Per-field statements embed the
    /// control_filter so the caller only modifies rows they own. Returns `true`
    /// when at least one field was updated; returns `Ok(true)` when `patch` is
    /// empty (no writes occurred but the caller already read-checked the task,
    /// so the phantom "succeeded" is acceptable — callers that reach here with
    /// an empty patch performed a read first).
    async fn update_task(
        &self,
        task_id: &str,
        patch: &TaskPatch,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;
    /// Flip status to `new_status` only if the task is not already `Closed`.
    ///
    /// Returns `true` when a row changed (status updated), `false` when the
    /// task was already `Closed` (no-op). The `WHERE status <> 'closed'` guard
    /// is evaluated atomically at the DB, so this closes the close-vs-terminal
    /// TOCTOU a plain read-then-write would lose: a `Closed` task can never be
    /// resurrected by an automated agent-lifecycle event. `Open`→`Open` and
    /// `Error`→`Open` still match the predicate, preserving today's behaviour.
    ///
    /// System callers (reaper, event handlers) pass `ScopeContext::system()` so
    /// they are not limited to the task owner.
    async fn update_status_if_not_closed(
        &self,
        task_id: &str,
        new_status: TaskStatus,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;
    /// Delete a task. Returns `true` when a row was deleted, `false` when the
    /// task was not found or the caller does not have control (existence hiding).
    async fn delete_task(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;
    async fn exists_by_short_id(&self, short_id: &str) -> Result<bool, sqlx::Error>;
    async fn get_created_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;
    async fn get_sandbox_started_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;
    /// Batch-fetch the durable Sandbox lifecycle state (desired state, stop
    /// reason, start + intent timestamps) for the given tasks from `sandboxes`.
    /// Existing Tasks with no Sandbox row are returned with the lifecycle
    /// module's lazy converged-Stopped default.
    async fn get_sandbox_intents(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, SandboxIntentView>, sqlx::Error>;
    /// Batch-fetch `ttl_minutes` for the given tasks. Tasks with no row are
    /// omitted (not present in the map).
    async fn get_ttl_minutes_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error>;
    /// Batch-fetch `inactive_ttl_days` for the given tasks. Tasks with no row
    /// are omitted (not present in the map).
    async fn get_inactive_ttl_days_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error>;
    /// List task IDs of all tasks currently in `status = 'open'`.
    async fn list_open_task_ids(&self) -> Result<Vec<String>, sqlx::Error>;

    /// Upsert a user's read marker for a task. Monotonic — never moves the
    /// stored `last_read_at` backwards, so out-of-order calls are safe.
    async fn mark_task_read(
        &self,
        user_id: &str,
        task_id: &str,
        read_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    /// Batch-fetch a user's `last_read_at` for the given tasks. Tasks the user
    /// has never opened are omitted (not present in the map).
    async fn get_read_receipts(
        &self,
        user_id: &str,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;

    /// Make `user_id` follow `task_id`. Idempotent — following an
    /// already-followed task is a no-op.
    async fn follow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error>;

    /// Make `user_id` stop following `task_id`. Idempotent — unfollowing a
    /// task that isn't followed is a no-op.
    async fn unfollow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error>;

    /// Batch-fetch the subset of `task_ids` that `user_id` currently follows.
    async fn get_followed(
        &self,
        user_id: &str,
        task_ids: &[String],
    ) -> Result<std::collections::HashSet<String>, sqlx::Error>;

    /// List task IDs for currently-open codex tasks under a given codex scope,
    /// optionally excluding `except_task_id`. Drives both the Codex refresh
    /// in-flight guard (empty → backend may refresh; non-empty → defer to the
    /// active sandbox watcher) and the rotation broadcast fan-out.
    ///
    /// `scope_kind` is `"user"` or `"global"`. `scope_id` is the user_id;
    /// ignored for global. Pass `""` for `except_task_id` to disable the
    /// exclusion. Owned `String` params keep `mockall::automock` happy —
    /// `Option<&str>` and `&str` elide lifetimes the macro can't synthesise.
    async fn list_open_codex_tasks_for_scope(
        &self,
        scope_kind: String,
        scope_id: String,
        except_task_id: String,
    ) -> Result<Vec<String>, sqlx::Error>;
}

/// PostgreSQL implementation of [`TaskRepository`].
pub struct PgTaskRepository(pub DbPool);

#[async_trait]
impl TaskRepository for PgTaskRepository {
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Task, "", 2);
        let sql = format!(
            "SELECT * FROM tasks WHERE task_id = $1 AND {}",
            access.sql(),
        );
        sqlx::query_as::<_, Task>(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn insert_task(&self, row: &Task) -> Result<(), sqlx::Error> {
        // Resolve the workspace this task belongs to. Callers may set
        // `workspace_id` explicitly (future multi-repo flows); when unset, ensure
        // the repository's deterministic shadow workspace exists and use it — so
        // every creation path (HTTP handler, adapters, triggers) persists a
        // workspace_id consistently, exactly as the Phase A backfill did for
        // pre-existing tasks. Applied here at the DAO layer, mirroring the
        // auto-follow below, so no creation path can silently skip it.
        let workspace_id = match row.workspace_id.clone() {
            Some(id) => id,
            None => {
                crate::workspace::dao::ensure_shadow_workspace(&self.0, &row.repository_id).await?
            }
        };

        // Durable Sandbox state lives in `sandboxes`, born lazily through
        // lifecycle — task creation writes none of it.
        sqlx::query(
            r#"INSERT INTO tasks (
                task_id, short_id, created_at, last_message_at, agent_type,
                repository_id, status, total_cost_usd, site_url, template_id,
                vibe, name, description, site_id, site_provider, branch,
                created_by, visibility, scope, owner_user_id,
                append_system_prompt, updated_at, headless, ttl_minutes,
                inactive_ttl_days, trigger_id, disabled_servers,
                workspace_id, server_overrides
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                $21, NOW(), $22, $23, $24, $25, $26,
                $27, $28
            )"#,
        )
        .bind(&row.task_id)
        .bind(&row.short_id)
        .bind(&row.created_at)
        .bind(&row.last_message_at)
        .bind(&row.agent_type)
        .bind(&row.repository_id)
        .bind(row.status)
        .bind(row.total_cost_usd)
        .bind(&row.site_url)
        .bind(&row.template_id)
        .bind(row.vibe)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.site_id)
        .bind(&row.site_provider)
        .bind(&row.branch)
        .bind(&row.created_by)
        .bind(row.visibility)
        .bind(row.scope)
        .bind(&row.owner_user_id)
        .bind(&row.append_system_prompt)
        .bind(row.headless)
        .bind(row.ttl_minutes)
        .bind(row.inactive_ttl_days)
        .bind(&row.trigger_id)
        .bind(&row.disabled_servers)
        .bind(&workspace_id)
        .bind(&row.server_overrides)
        .execute(&self.0)
        .await?;

        // Auto-follow the task for its creator so it lands in their personal
        // task list. Applied at the DAO layer so every creation path (HTTP
        // handler, adapters, triggers) follows consistently. Best-effort — a
        // follow failure must never fail task insertion.
        if let Some(ref creator) = row.created_by {
            if let Err(e) = self.follow_task(creator, &row.task_id).await {
                tracing::warn!(
                    error = %e,
                    task_id = %row.task_id,
                    "Failed to auto-follow task for creator"
                );
            }
        }

        Ok(())
    }

    async fn list_tasks(&self, filter: &TaskFilter) -> Result<(Vec<Task>, i64), sqlx::Error> {
        // One row of the joined result set: the total, plus the page columns.
        //
        // Every page column is optional because the query always emits the
        // total, even when the requested offset lies past the end of the
        // filtered relation — that case comes back as a single row whose page
        // columns are all NULL. See [`TaskPageRow::into_task`].
        #[derive(sqlx::FromRow)]
        struct TaskPageRow {
            total_count: i64,
            task_id: Option<String>,
            short_id: Option<String>,
            created_at: Option<String>,
            last_message_at: Option<String>,
            agent_type: Option<AgentType>,
            repository_id: Option<String>,
            workspace_id: Option<String>,
            status: Option<TaskStatus>,
            total_cost_usd: Option<f64>,
            site_url: Option<String>,
            template_id: Option<String>,
            vibe: Option<bool>,
            name: Option<String>,
            description: Option<String>,
            site_id: Option<String>,
            site_provider: Option<String>,
            branch: Option<String>,
            created_by: Option<String>,
            visibility: Option<Visibility>,
            scope: Option<Scope>,
            owner_user_id: Option<String>,
            created_at_db: Option<DateTime<Utc>>,
            updated_at: Option<DateTime<Utc>>,
            append_system_prompt: Option<String>,
            headless: Option<bool>,
            ttl_minutes: Option<i32>,
            inactive_ttl_days: Option<i32>,
            trigger_id: Option<String>,
            disabled_servers: Option<serde_json::Value>,
            server_overrides: Option<serde_json::Value>,
        }

        impl TaskPageRow {
            /// The total-only row the query emits when the page is empty, which
            /// carries the total but no task.
            fn is_total_only(&self) -> bool {
                self.task_id.is_none()
            }

            /// `None` only when a column that is `NOT NULL` in `tasks` came back
            /// NULL. Callers must reject that rather than skip the row: dropping
            /// a task from the page while still counting it in the total is the
            /// page/total drift this query exists to prevent.
            fn into_task(self) -> Option<Task> {
                Some(Task {
                    task_id: self.task_id?,
                    short_id: self.short_id,
                    created_at: self.created_at?,
                    last_message_at: self.last_message_at?,
                    agent_type: self.agent_type?,
                    repository_id: self.repository_id?,
                    workspace_id: self.workspace_id,
                    status: self.status?,
                    total_cost_usd: self.total_cost_usd?,
                    site_url: self.site_url,
                    template_id: self.template_id,
                    vibe: self.vibe,
                    name: self.name,
                    description: self.description,
                    site_id: self.site_id,
                    site_provider: self.site_provider,
                    branch: self.branch,
                    created_by: self.created_by,
                    visibility: self.visibility?,
                    scope: self.scope?,
                    owner_user_id: self.owner_user_id,
                    created_at_db: self.created_at_db,
                    updated_at: self.updated_at,
                    append_system_prompt: self.append_system_prompt,
                    headless: self.headless?,
                    ttl_minutes: self.ttl_minutes?,
                    inactive_ttl_days: self.inactive_ttl_days?,
                    trigger_id: self.trigger_id,
                    disabled_servers: self.disabled_servers,
                    server_overrides: self.server_overrides,
                })
            }
        }

        let access = filter.scope.access_filter(ScopedEntity::Task, "t.", 1);
        // `filtered` is the single source of truth for both the page and the
        // total: `total` counts it and `page` slices it, so the two can never
        // drift apart under a filter or access-control change.
        //
        // It holds exactly one row per matching task. The tag, parent-relation
        // and followed-by filters are all `EXISTS` subqueries rather than joins,
        // so a task carrying several tags is not fanned out into several rows
        // (which is what made the old `COUNT(*) OVER()` over-count). When a
        // filter is unset its subquery is skipped entirely (`$n IS NULL`).
        //
        // `total LEFT JOIN page ON TRUE` is what keeps the total available when
        // the page is empty: `total` is an ungrouped aggregate and therefore
        // always yields exactly one row, so an offset past the end returns that
        // row with NULL page columns rather than returning nothing at all.
        let sql = format!(
            r#"WITH filtered AS (
                SELECT
                    t.task_id, t.short_id, t.created_at, t.last_message_at, t.agent_type,
                    t.repository_id, t.workspace_id, t.status, t.total_cost_usd, t.site_url, t.template_id,
                    t.vibe, t.name, t.description, t.site_id, t.site_provider, t.branch,
                    t.created_by, t.visibility, t.scope, t.owner_user_id,
                    t.created_at_db, t.updated_at, t.append_system_prompt,
                    t.headless, t.ttl_minutes,
                    t.inactive_ttl_days, t.trigger_id, t.disabled_servers,
                    t.server_overrides
                FROM tasks t
                WHERE {}
                  AND ($3::text IS NULL OR t.status = $3)
                  AND ($10::text IS NULL OR t.status != $10)
                  AND ($4::text IS NULL OR t.repository_id = $4)
                  AND ($5::text IS NULL OR t.created_by = $5)
                  AND ($6::text IS NULL OR EXISTS (
                      SELECT 1 FROM task_tags tt
                      JOIN tags tg ON tg.tag_id = tt.tag_id
                      WHERE tt.task_id = t.task_id
                        AND tg.name = $6
                  ))
                  AND ($7::text IS NULL OR EXISTS (
                      SELECT 1 FROM task_relations tr
                      WHERE tr.from_task_id = t.task_id
                        AND tr.relation_type = 'created_by'
                        AND tr.to_task_id = $7
                  ))
                  AND ($11::text IS NULL OR EXISTS (
                      SELECT 1 FROM task_follows tf
                      WHERE tf.task_id = t.task_id
                        AND tf.user_id = $11
                  ))
            ),
            total AS (SELECT COUNT(*) AS total_count FROM filtered),
            page AS (
                SELECT * FROM filtered
                ORDER BY updated_at DESC NULLS LAST
                LIMIT $8 OFFSET $9
            )
            SELECT tot.total_count, p.*
            FROM total tot
            LEFT JOIN page p ON TRUE
            ORDER BY p.updated_at DESC NULLS LAST"#,
            access.sql(),
        );
        let rows = sqlx::query_as::<_, TaskPageRow>(AssertSqlSafe(sql))
            .bind(filter.scope.is_admin)
            .bind(&filter.scope.user_id)
            .bind(filter.status)
            .bind(&filter.repository_id)
            .bind(&filter.created_by)
            .bind(&filter.tag)
            .bind(&filter.parent_task_id)
            .bind(filter.limit)
            .bind(filter.offset)
            .bind(filter.exclude_status)
            .bind(&filter.followed_by)
            .fetch_all(&self.0)
            .await?;

        let total = rows.first().map_or(0, |r| r.total_count);

        // Skip only the total-only row. A conversion that fails for any other
        // reason means a `NOT NULL` column of `tasks` came back NULL, which
        // must fail loudly — silently skipping it would drop a task from the
        // page while leaving it counted in the total.
        let mut tasks: Vec<Task> = Vec::with_capacity(rows.len());
        for row in rows {
            if row.is_total_only() {
                continue;
            }
            tasks.push(row.into_task().ok_or_else(|| {
                sqlx::Error::Decode("task row has a NULL in a NOT NULL column".into())
            })?);
        }

        Ok((tasks, total))
    }

    async fn update_task(
        &self,
        task_id: &str,
        patch: &TaskPatch,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        // Use individual UPDATE statements per field for type safety.
        // Each statement embeds the control_filter so only the owner (or admin)
        // can modify the row. The first executed statement's rows_affected drives
        // the return value; subsequent statements return their own rows_affected
        // but we track the first non-empty patch field's result.
        let ctrl = ctx.control_filter("", 3);
        let ctrl_sql = ctrl.sql();
        let mut first_affected: Option<u64> = None;

        macro_rules! run_field {
            ($set_clause:expr, $value:expr) => {{
                let sql = format!(
                    "UPDATE tasks SET {}, updated_at = NOW() WHERE task_id = $1 AND {}",
                    $set_clause, ctrl_sql
                );
                let res = sqlx::query(AssertSqlSafe(sql))
                    .bind(task_id)
                    .bind($value)
                    .bind(ctx.is_admin)
                    .bind(&ctx.user_id)
                    .execute(&self.0)
                    .await?;
                if first_affected.is_none() {
                    first_affected = Some(res.rows_affected());
                }
            }};
        }

        if let Some(ref status) = patch.status {
            run_field!("status = $2", status);
        }
        if let Some(ref name) = patch.name {
            run_field!("name = $2", name);
        }
        if let Some(ref description) = patch.description {
            run_field!("description = $2", description);
        }
        if let Some(ref last_message_at) = patch.last_message_at {
            run_field!("last_message_at = $2", last_message_at);
        }
        if let Some(total_cost_usd) = patch.total_cost_usd {
            run_field!("total_cost_usd = $2", total_cost_usd);
        }
        if let Some(increment) = patch.total_cost_usd_increment {
            let sql = format!(
                "UPDATE tasks SET total_cost_usd = total_cost_usd + $2, updated_at = NOW() \
                 WHERE task_id = $1 AND {ctrl_sql}"
            );
            let res = sqlx::query(AssertSqlSafe(sql))
                .bind(task_id)
                .bind(increment)
                .bind(ctx.is_admin)
                .bind(&ctx.user_id)
                .execute(&self.0)
                .await?;
            if first_affected.is_none() {
                first_affected = Some(res.rows_affected());
            }
        }
        if let Some(ref site_url) = patch.site_url {
            run_field!("site_url = $2", site_url);
        }
        if let Some(ref visibility) = patch.visibility {
            run_field!("visibility = $2", visibility);
        }
        if let Some(ref short_id) = patch.short_id {
            run_field!("short_id = $2", short_id);
        }
        if let Some(ref append_system_prompt) = patch.append_system_prompt {
            run_field!("append_system_prompt = $2", append_system_prompt);
        }
        if let Some(ttl_minutes) = patch.ttl_minutes {
            run_field!("ttl_minutes = $2", ttl_minutes);
        }
        if let Some(inactive_ttl_days) = patch.inactive_ttl_days {
            run_field!("inactive_ttl_days = $2", inactive_ttl_days);
        }
        if let Some(ref disabled_servers) = patch.disabled_servers {
            // Full replace — the column always holds the complete disabled set.
            let json = serde_json::json!(disabled_servers);
            run_field!("disabled_servers = $2", json);
        }
        if let Some(ref server_overrides) = patch.server_overrides {
            // Full replace — the column always holds the complete override map.
            run_field!("server_overrides = $2", server_overrides);
        }
        // Empty patch: caller read-checked the task first; return true.
        Ok(first_affected.is_none_or(|n| n > 0))
    }

    async fn update_status_if_not_closed(
        &self,
        task_id: &str,
        new_status: TaskStatus,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        let ctrl = ctx.control_filter("", 3);
        let sql = format!(
            "UPDATE tasks SET status = $2, updated_at = NOW() \
             WHERE task_id = $1 AND status <> 'closed' AND {}",
            ctrl.sql()
        );
        let res = sqlx::query(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(new_status)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn delete_task(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let ctrl = ctx.control_filter("", 2);
        let sql = format!(
            "DELETE FROM tasks WHERE task_id = $1 AND {}",
            ctrl.sql()
        );
        let res = sqlx::query(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn exists_by_short_id(&self, short_id: &str) -> Result<bool, sqlx::Error> {
        let row: (bool,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM tasks WHERE short_id = $1)")
            .bind(short_id)
            .fetch_one(&self.0)
            .await?;
        Ok(row.0)
    }

    async fn get_created_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT task_id, COALESCE(created_at_db, NOW()) AS created_at
               FROM tasks WHERE task_id = ANY($1)"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn get_sandbox_started_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT task_id, started_at
               FROM sandboxes
               WHERE task_id = ANY($1) AND started_at IS NOT NULL"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn get_sandbox_intents(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, SandboxIntentView>, sqlx::Error> {
        load_sandbox_intent_views(&self.0, task_ids).await
    }

    async fn get_ttl_minutes_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, i32)> =
            sqlx::query_as(r#"SELECT task_id, ttl_minutes FROM tasks WHERE task_id = ANY($1)"#)
                .bind(task_ids)
                .fetch_all(&self.0)
                .await?;

        Ok(rows.into_iter().collect())
    }

    async fn get_inactive_ttl_days_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, i32)> = sqlx::query_as(
            r#"SELECT task_id, inactive_ttl_days FROM tasks WHERE task_id = ANY($1)"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn list_open_task_ids(&self) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> =
            sqlx::query_as(r#"SELECT task_id FROM tasks WHERE status = 'open'"#)
                .fetch_all(&self.0)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn mark_task_read(
        &self,
        user_id: &str,
        task_id: &str,
        read_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO task_read_receipts (user_id, task_id, last_read_at)
               VALUES ($1, $2, $3)
               ON CONFLICT (user_id, task_id)
               DO UPDATE SET last_read_at = EXCLUDED.last_read_at
               WHERE task_read_receipts.last_read_at < EXCLUDED.last_read_at"#,
        )
        .bind(user_id)
        .bind(task_id)
        .bind(read_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn get_read_receipts(
        &self,
        user_id: &str,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT task_id, last_read_at
               FROM task_read_receipts
               WHERE user_id = $1 AND task_id = ANY($2)"#,
        )
        .bind(user_id)
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn follow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO task_follows (user_id, task_id)
               VALUES ($1, $2)
               ON CONFLICT (user_id, task_id) DO NOTHING"#,
        )
        .bind(user_id)
        .bind(task_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn unfollow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(r#"DELETE FROM task_follows WHERE user_id = $1 AND task_id = $2"#)
            .bind(user_id)
            .bind(task_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn get_followed(
        &self,
        user_id: &str,
        task_ids: &[String],
    ) -> Result<std::collections::HashSet<String>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }

        let rows: Vec<(String,)> = sqlx::query_as(
            r#"SELECT task_id
               FROM task_follows
               WHERE user_id = $1 AND task_id = ANY($2)"#,
        )
        .bind(user_id)
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn list_open_codex_tasks_for_scope(
        &self,
        scope_kind: String,
        scope_id: String,
        except_task_id: String,
    ) -> Result<Vec<String>, sqlx::Error> {
        let except = if except_task_id.is_empty() {
            None
        } else {
            Some(except_task_id.as_str())
        };
        let rows: Vec<(String,)> = match scope_kind.as_str() {
            "user" => {
                sqlx::query_as(
                    r#"SELECT task_id FROM tasks
                       WHERE status = 'open'
                         AND agent_type = 'codex'
                         AND created_by = $1
                         AND ($2::text IS NULL OR task_id <> $2)"#,
                )
                .bind(&scope_id)
                .bind(except)
                .fetch_all(&self.0)
                .await?
            }
            "global" => {
                sqlx::query_as(
                    r#"SELECT task_id FROM tasks
                       WHERE status = 'open'
                         AND agent_type = 'codex'
                         AND ($1::text IS NULL OR task_id <> $1)"#,
                )
                .bind(except)
                .fetch_all(&self.0)
                .await?
            }
            _ => Vec::new(),
        };
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}
/// Patch struct for partial task updates.
#[derive(Default)]
pub struct TaskPatch {
    pub status: Option<TaskStatus>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub last_message_at: Option<String>,
    pub total_cost_usd: Option<f64>,
    /// Additive increment to total_cost_usd (uses SQL `total_cost_usd + $N`).
    pub total_cost_usd_increment: Option<f64>,
    pub site_url: Option<String>,
    pub visibility: Option<Visibility>,
    pub short_id: Option<String>,
    pub append_system_prompt: Option<Option<String>>,
    pub ttl_minutes: Option<i32>,
    pub inactive_ttl_days: Option<i32>,
    /// Full-replace the per-task auto-disabled server list.
    pub disabled_servers: Option<Vec<String>>,
    /// Full-replace the per-task live server overrides (see
    /// [`crate::task::models::Task::server_overrides`]).
    pub server_overrides: Option<serde_json::Value>,
}

// =========================================================================
// Free-function wrappers for integration tests
// =========================================================================
//
// Integration tests in `tests/` call these as `task_dao::insert_task(&pool, &row)`.
// Each wrapper constructs a `PgTaskRepository` and delegates to the trait method.

/// Insert a task (convenience wrapper).
pub async fn insert_task(pool: &DbPool, row: &Task) -> Result<(), sqlx::Error> {
    PgTaskRepository(pool.clone()).insert_task(row).await
}

/// Find by task_id scoped to access control (convenience wrapper).
pub async fn find_by_id(
    pool: &DbPool,
    task_id: &str,
    ctx: &ScopeContext,
) -> Result<Option<Task>, sqlx::Error> {
    PgTaskRepository(pool.clone())
        .find_by_id(task_id, ctx)
        .await
}

/// List tasks with individual filter arguments (convenience wrapper).
#[allow(clippy::too_many_arguments)]
pub async fn list_tasks(
    pool: &DbPool,
    ctx: &ScopeContext,
    limit: i64,
    offset: i64,
    status: Option<TaskStatus>,
    repository_id: Option<&str>,
    created_by: Option<&str>,
    tag: Option<&str>,
    parent_task_id: Option<&str>,
) -> Result<(Vec<Task>, i64), sqlx::Error> {
    let filter = TaskFilter {
        scope: ctx.clone(),
        limit,
        offset,
        status,
        exclude_status: None,
        repository_id: repository_id.map(String::from),
        created_by: created_by.map(String::from),
        tag: tag.map(String::from),
        parent_task_id: parent_task_id.map(String::from),
        followed_by: None,
    };
    PgTaskRepository(pool.clone()).list_tasks(&filter).await
}

/// Update task fields (convenience wrapper).
pub async fn update_task(
    pool: &DbPool,
    task_id: &str,
    patch: &TaskPatch,
    ctx: &ScopeContext,
) -> Result<bool, sqlx::Error> {
    PgTaskRepository(pool.clone())
        .update_task(task_id, patch, ctx)
        .await
}

/// Delete a task (convenience wrapper).
pub async fn delete_task(
    pool: &DbPool,
    task_id: &str,
    ctx: &ScopeContext,
) -> Result<bool, sqlx::Error> {
    PgTaskRepository(pool.clone())
        .delete_task(task_id, ctx)
        .await
}

/// Check if a task with the given short_id exists (convenience wrapper).
pub async fn exists_by_short_id(pool: &DbPool, short_id: &str) -> Result<bool, sqlx::Error> {
    PgTaskRepository(pool.clone())
        .exists_by_short_id(short_id)
        .await
}
