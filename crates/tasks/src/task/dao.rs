//! Task DAO — queries for task data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use super::models::{
    sandbox_intent_epoch, SandboxDesiredState, SandboxIntentView, SandboxStopReason, Task,
    TaskStatus, Visibility,
};
use moatless_backend_core::scope::{Scope, ScopeContext, ScopedEntity};
use moatless_core::db::DbPool;
use moatless_core::shared_models::AgentType;
use crate::task_listing;

#[derive(sqlx::FromRow)]
struct SandboxIntentViewRow {
    task_id: String,
    started_at: Option<DateTime<Utc>>,
    desired_state: Option<SandboxDesiredState>,
    stop_reason: Option<SandboxStopReason>,
    desired_state_updated_at: Option<DateTime<Utc>>,
}

/// Batch-load the task-response projection of Sandbox intent from the
/// `sandboxes` table. Owns the lazy-row semantic: an existing Task without a
/// Sandbox row is returned as converged Stopped at the no-intent epoch. Lives
/// here beside the other task-response reads of the `sandboxes` table (the
/// projection type and epoch sentinel are tasks-band); `Lifecycle` remains the
/// sole *writer* of that intent.
async fn load_sandbox_intent_views(
    pool: &DbPool,
    task_ids: &[String],
) -> Result<HashMap<String, SandboxIntentView>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<SandboxIntentViewRow> = sqlx::query_as(
        r#"SELECT t.task_id, s.started_at, s.desired_state, s.stop_reason,
                  s.desired_state_updated_at
           FROM tasks t
           LEFT JOIN sandboxes s ON s.task_id = t.task_id
           WHERE t.task_id = ANY($1)"#,
    )
    .bind(task_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.task_id,
                SandboxIntentView {
                    started_at: row.started_at,
                    desired_state: row.desired_state.unwrap_or(SandboxDesiredState::Stopped),
                    stop_reason: row.stop_reason,
                    desired_state_updated_at: row
                        .desired_state_updated_at
                        .unwrap_or_else(sandbox_intent_epoch),
                },
            )
        })
        .collect())
}

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

/// DAO trait for task persistence.
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait]
pub trait TaskDao: Send + Sync {
    /// Find by task_id, scoped to access control.
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error>;
    /// Insert a task, returning the workspace id the row was persisted with.
    /// That is `row.workspace_id` when the caller set one, otherwise the
    /// repository's shadow workspace this resolves — callers building a response
    /// from the in-memory row need it to label the workspace.
    async fn insert_task(&self, row: &Task) -> Result<String, sqlx::Error>;
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

    async fn settle_task(
        &self,
        user_id: &str,
        task_id: &str,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    async fn unsettle_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error>;

    async fn snooze_task(
        &self,
        user_id: &str,
        task_id: &str,
        until: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    async fn unsnooze_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error>;

    async fn clear_snoozed_for_follower(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<(), sqlx::Error>;

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

/// PostgreSQL implementation of [`TaskDao`].
pub struct PgTaskDao(pub DbPool);

/// The E-(b) session-gate port over `TaskDao`: `verify_handler` (identity) holds
/// `Arc<dyn TaskAccess>`, not a `TaskDao`, so identity keeps no `task` import.
#[async_trait]
impl moatless_identity::extractors::TaskAccess for PgTaskDao {
    async fn may_access_task(&self, task_id: &str, ctx: &ScopeContext) -> bool {
        // `find_by_id` is already scope-filtered, so a task `ctx` cannot see
        // returns `None`. Fail-closed on a DB error, matching the handler's prior
        // `map_err(|_| Forbidden)`.
        matches!(self.find_by_id(task_id, ctx).await, Ok(Some(_)))
    }
}

#[async_trait]
impl TaskDao for PgTaskDao {
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

    async fn insert_task(&self, row: &Task) -> Result<String, sqlx::Error> {
        // Resolve the workspace this task belongs to. Callers may set
        // `workspace_id` explicitly (future multi-repo flows); when unset, ensure
        // the repository's deterministic shadow workspace exists and use it — so
        // every creation path (HTTP handler, adapters, loops) persists a
        // workspace_id consistently, exactly as the Phase A backfill did for
        // pre-existing tasks. Applied here at the DAO layer, mirroring the
        // auto-follow below, so no creation path can silently skip it.
        let workspace_id = match row.workspace_id.clone() {
            Some(id) => id,
            None => {
                moatless_workspaces::workspace::dao::ensure_shadow_workspace(&self.0, &row.repository_id).await?
            }
        };

        // Durable Sandbox state lives in `sandboxes`, born lazily through
        // lifecycle — task creation writes none of it.
        let mut tx = self.0.begin().await?;
        sqlx::query(
            r#"INSERT INTO tasks (
                task_id, short_id, created_at, last_message_at, agent_type,
                repository_id, status, total_cost_usd, site_url, template_id,
                vibe, name, description, site_id, site_provider, branch,
                created_by, visibility, scope, owner_user_id,
                append_system_prompt, updated_at, headless, ttl_minutes,
                inactive_ttl_days, loop_id, disabled_servers, model,
                workspace_id, skills
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                $21, NOW(), $22, $23, $24, $25, $26,
                $27, $28, $29
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
        .bind(&row.loop_id)
        .bind(&row.disabled_servers)
        .bind(&row.model)
        .bind(&workspace_id)
        .bind(&row.skills)
        .execute(&mut *tx)
        .await?;

        // A conversation coming into existence is a listing change. Last, after
        // the `tasks` insert — `record_change`'s "call this last".
        task_listing::record_change(&mut tx, &row.task_id).await?;
        tx.commit().await?;

        // Auto-follow the task for its creator so it lands in their personal
        // task list. Applied at the DAO layer so every creation path (HTTP
        // handler, adapters, loops) follows consistently. Best-effort — a
        // follow failure must never fail task insertion.
        //
        // Deliberately after the commit rather than inside it: an error inside a
        // transaction aborts it, so a "best-effort" write that cannot fail the
        // insert cannot live in the insert's transaction. It records its own
        // change, and the two coalesce into one delivered row.
        if let Some(ref creator) = row.created_by {
            if let Err(e) = self.follow_task(creator, &row.task_id).await {
                tracing::warn!(
                    error = %e,
                    task_id = %row.task_id,
                    "Failed to auto-follow task for creator"
                );
            }
        }

        Ok(workspace_id)
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
            model: Option<String>,
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
            loop_id: Option<String>,
            disabled_servers: Option<serde_json::Value>,
            skills: Option<Vec<String>>,
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
                    model: self.model,
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
                    loop_id: self.loop_id,
                    disabled_servers: self.disabled_servers,
                    // NOT NULL in `tasks`, so only the total-only row can be
                    // NULL here — and that row is rejected before this point.
                    skills: self.skills.unwrap_or_default(),
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
                    t.model, t.created_by, t.visibility, t.scope, t.owner_user_id,
                    t.created_at_db, t.updated_at, t.append_system_prompt,
                    t.headless, t.ttl_minutes,
                    t.inactive_ttl_days, t.loop_id, t.disabled_servers, t.skills
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
        //
        // One transaction across the fields, because a listing change and the
        // mutation it describes have to commit together or not at all. Every
        // statement here shares one `WHERE`, so they all match the same row and
        // `first_affected` speaks for all of them.
        let mut tx = self.0.begin().await?;
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
                    .execute(&mut *tx)
                    .await?;
                if first_affected.is_none() {
                    first_affected = Some(res.rows_affected());
                }
            }};
        }

        if let Some(ref status) = patch.status {
            // `closed_at` is a fact of `status`, so the write that sets one
            // sets the other. `COALESCE` keeps the original time on a
            // close-to-close write: a task already closed was closed then.
            run_field!(
                "status = $2, closed_at = CASE \
                 WHEN $2 = 'closed' THEN COALESCE(closed_at, NOW()) ELSE NULL END",
                status
            );
        }
        if let Some(ref name) = patch.name {
            run_field!("name = $2", name);
        }
        if let Some(ref branch) = patch.branch {
            run_field!("branch = $2", branch);
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
                .execute(&mut *tx)
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
        if let Some(ref model) = patch.model {
            run_field!("model = $2", model);
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
        // Last, after every `tasks` statement above — `record_change`'s "call
        // this last". Gated twice: on `rows_affected`, because a patch the
        // caller may not control moves nothing; and on `touches_listing`,
        // because most of this patch is not sidebar-visible and the busiest of
        // those paths runs once per turn.
        if patch.touches_listing() && first_affected.is_some_and(|n| n > 0) {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;

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
        // `closed_at` is a fact of `status` — the same rule `update_task`
        // applies. Without it this path leaves the two disagreeing: a task
        // closed here would be archived with no time to show for it, and a
        // reopen here would keep the old close time and stay in the archive.
        let sql = format!(
            "UPDATE tasks SET status = $2, \
             closed_at = CASE WHEN $2 = 'closed' THEN COALESCE(closed_at, NOW()) ELSE NULL END, \
             updated_at = NOW() \
             WHERE task_id = $1 AND status <> 'closed' AND {}",
            ctrl.sql()
        );
        let mut tx = self.0.begin().await?;
        let res = sqlx::query(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(new_status)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&mut *tx)
            .await?;
        // Only when the guard let the flip through: a task already `closed`
        // matches nothing here, and a no-op must not advance the position.
        // Last, after the `tasks` row lock — `record_change`'s "call this last".
        if res.rows_affected() > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    async fn delete_task(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let ctrl = ctx.control_filter("", 2);
        let sql = format!("DELETE FROM tasks WHERE task_id = $1 AND {}", ctrl.sql());
        let mut tx = self.0.begin().await?;
        let res = sqlx::query(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&mut *tx)
            .await?;
        // The record outlives the Task it names — `task_listing_changes` has no
        // foreign key precisely so that a deletion is still replayable. A
        // watching client re-reads this id, finds nothing, and drops the row it
        // holds.
        if res.rows_affected() > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
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
        let mut tx = self.0.begin().await?;
        let res = sqlx::query(
            r#"INSERT INTO task_follows (user_id, task_id)
               VALUES ($1, $2)
               ON CONFLICT (user_id, task_id) DO NOTHING"#,
        )
        .bind(user_id)
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
        // Following is the one change that alters the *membership* of a sidebar
        // rather than the contents of a row already in it, so a client that
        // handles everything else can still miss this one. `ON CONFLICT DO
        // NOTHING` makes the call idempotent; recording on the effect rather
        // than the call keeps a re-follow from advancing the position.
        if res.rows_affected() > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn unfollow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let res = sqlx::query(r#"DELETE FROM task_follows WHERE user_id = $1 AND task_id = $2"#)
            .bind(user_id)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        // The row leaves this person's listing and nobody else's. The delivery
        // path re-reads it per viewer, so the one whose follow this was finds
        // nothing and the rest see no change at all.
        if res.rows_affected() > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn settle_task(
        &self,
        user_id: &str,
        task_id: &str,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let res = sqlx::query(
            r#"UPDATE task_follows
                  SET settled_override = 'settled',
                      settled_at = $3
                WHERE user_id = $1 AND task_id = $2
                  AND settled_override IS DISTINCT FROM 'settled'"#,
        )
        .bind(user_id)
        .bind(task_id)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn unsettle_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let res = sqlx::query(
            r#"UPDATE task_follows
                  SET settled_override = 'active',
                      settled_at = NULL
                WHERE user_id = $1 AND task_id = $2
                  AND (settled_override IS DISTINCT FROM 'active' OR settled_at IS NOT NULL)"#,
        )
        .bind(user_id)
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn snooze_task(
        &self,
        user_id: &str,
        task_id: &str,
        until: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let res = sqlx::query(
            r#"UPDATE task_follows
                  SET snoozed_until = $3,
                      snoozed_at = $4
                WHERE user_id = $1 AND task_id = $2
                  AND snoozed_until IS DISTINCT FROM $3"#,
        )
        .bind(user_id)
        .bind(task_id)
        .bind(until)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn unsnooze_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let res = sqlx::query(
            r#"UPDATE task_follows
                  SET snoozed_until = NULL,
                      snoozed_at = NULL
                WHERE user_id = $1 AND task_id = $2
                  AND (snoozed_until IS NOT NULL OR snoozed_at IS NOT NULL)"#,
        )
        .bind(user_id)
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn clear_snoozed_for_follower(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE task_follows
                  SET snoozed_until = NULL,
                      snoozed_at = NULL
                WHERE user_id = $1 AND task_id = $2
                  AND (snoozed_until IS NOT NULL OR snoozed_at IS NOT NULL)"#,
        )
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

/// Clear `settled_override`/`settled_at` for every follower of `task_id`.
pub async fn clear_settled_for_all_followers(
    tx: &mut sqlx::PgConnection,
    task_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE task_follows
              SET settled_override = NULL,
                  settled_at = NULL
            WHERE task_id = $1
              AND (settled_override IS NOT NULL OR settled_at IS NOT NULL)"#,
    )
    .bind(task_id)
    .execute(tx)
    .await?;
    Ok(())
}
/// Patch struct for partial task updates.
#[derive(Default)]
pub struct TaskPatch {
    pub status: Option<TaskStatus>,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub description: Option<String>,
    pub last_message_at: Option<String>,
    pub total_cost_usd: Option<f64>,
    /// Additive increment to total_cost_usd (uses SQL `total_cost_usd + $N`).
    pub total_cost_usd_increment: Option<f64>,
    pub site_url: Option<String>,
    pub visibility: Option<Visibility>,
    pub short_id: Option<String>,
    pub model: Option<String>,
    pub append_system_prompt: Option<Option<String>>,
    pub ttl_minutes: Option<i32>,
    pub inactive_ttl_days: Option<i32>,
    /// Full-replace the per-task auto-disabled server list.
    pub disabled_servers: Option<Vec<String>>,
}

impl TaskPatch {
    /// Whether this patch moves anything a personal sidebar listing shows.
    ///
    /// The listed fields are the ones Decision 6 of the T3 shell projection spec
    /// puts in the change feed: a conversation being closed or reopened, its
    /// name or description, its visibility (which decides who may still read
    /// it), and its expiry. The rest — the conversation clock, the running cost,
    /// the site URL, the short id, the appended prompt, the disabled-server set
    /// — are written by paths a sidebar does not follow, and the busiest of them
    /// runs once per agent turn. Recording those would advance the deployment's
    /// one position counter to move values no sidebar carries.
    fn touches_listing(&self) -> bool {
        self.status.is_some()
            || self.name.is_some()
            || self.description.is_some()
            || self.visibility.is_some()
            || self.ttl_minutes.is_some()
            || self.inactive_ttl_days.is_some()
    }
}

// =========================================================================
// Free-function wrappers for integration tests
// =========================================================================
//
// Integration tests in `tests/` call these as `task_dao::insert_task(&pool, &row)`.
// Each wrapper constructs a `PgTaskDao` and delegates to the trait method.

/// Insert a task (convenience wrapper). Returns the workspace id the row was
/// persisted with, as the trait method does.
pub async fn insert_task(pool: &DbPool, row: &Task) -> Result<String, sqlx::Error> {
    PgTaskDao(pool.clone()).insert_task(row).await
}

/// Find by task_id scoped to access control (convenience wrapper).
pub async fn find_by_id(
    pool: &DbPool,
    task_id: &str,
    ctx: &ScopeContext,
) -> Result<Option<Task>, sqlx::Error> {
    PgTaskDao(pool.clone()).find_by_id(task_id, ctx).await
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
    PgTaskDao(pool.clone()).list_tasks(&filter).await
}

/// Update task fields (convenience wrapper).
pub async fn update_task(
    pool: &DbPool,
    task_id: &str,
    patch: &TaskPatch,
    ctx: &ScopeContext,
) -> Result<bool, sqlx::Error> {
    PgTaskDao(pool.clone())
        .update_task(task_id, patch, ctx)
        .await
}

/// Delete a task (convenience wrapper).
pub async fn delete_task(
    pool: &DbPool,
    task_id: &str,
    ctx: &ScopeContext,
) -> Result<bool, sqlx::Error> {
    PgTaskDao(pool.clone()).delete_task(task_id, ctx).await
}

/// Check if a task with the given short_id exists (convenience wrapper).
pub async fn exists_by_short_id(pool: &DbPool, short_id: &str) -> Result<bool, sqlx::Error> {
    PgTaskDao(pool.clone()).exists_by_short_id(short_id).await
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::task::test_support::seed_bare_task;
    use sqlx::PgPool;

    /// A Task with no `sandboxes` row projects as converged Stopped at the
    /// no-intent epoch — the lazy-row semantic this task-response projection
    /// owns. Deleting the projection would silently drop it.
    #[sqlx::test]
    async fn no_sandbox_row_projects_as_converged_stopped(pool: PgPool) {
        seed_bare_task(&pool, "t1", "open").await;

        let views = load_sandbox_intent_views(&pool, &[String::from("t1")])
            .await
            .unwrap();
        let view = views.get("t1").expect("existing task has a projection");
        assert_eq!(view.desired_state, SandboxDesiredState::Stopped);
        assert!(
            !view.has_explicit_intent(),
            "a no-row task must not look like an explicit stop"
        );
    }
}
