//! Task DAO — queries for task data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use super::models::{
    SandboxDesiredState, SandboxIntentView, SandboxStopReason, Task, TaskStatus, Visibility,
};
use crate::db::DbPool;
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
    /// Find by task_id with **no access control** — for system callers that
    /// have no authenticated user to derive a scope from (background jobs,
    /// reapers, pod watchers, adapter/webhook runtimes, sandbox-pod-facing gRPC)
    /// or intentionally privileged flows (e.g. the close cascade, which must
    /// reach child tasks in other users' scopes).
    ///
    /// Any user-facing handler MUST instead use [`Self::find_by_id`] with the
    /// caller's `ScopeContext`. When a system caller genuinely needs full
    /// access, `find_by_id(id, &ScopeContext::system())` is the equivalent
    /// scoped form and is preferred for new code.
    async fn find_by_id_internal(&self, task_id: &str) -> Result<Option<Task>, sqlx::Error>;
    /// Find by task_id, scoped to access control.
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error>;
    async fn insert_task(&self, row: &Task) -> Result<(), sqlx::Error>;
    async fn list_tasks(&self, filter: &TaskFilter) -> Result<(Vec<Task>, i64), sqlx::Error>;
    async fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), sqlx::Error>;
    /// Flip status to `new_status` only if the task is not already `Closed`.
    ///
    /// Returns `true` when a row changed (status updated), `false` when the
    /// task was already `Closed` (no-op). The `WHERE status <> 'closed'` guard
    /// is evaluated atomically at the DB, so this closes the close-vs-terminal
    /// TOCTOU a plain read-then-write would lose: a `Closed` task can never be
    /// resurrected by an automated agent-lifecycle event. `Open`→`Open` and
    /// `Error`→`Open` still match the predicate, preserving today's behaviour.
    async fn update_status_if_not_closed(
        &self,
        task_id: &str,
        new_status: TaskStatus,
    ) -> Result<bool, sqlx::Error>;
    async fn delete_task(&self, task_id: &str) -> Result<(), sqlx::Error>;
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
    /// Tasks with no Sandbox row are omitted — the caller applies the lazy
    /// converged-Stopped default.
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

    /// Batch-fetch the `status` of the given tasks. Tasks with no row are
    /// omitted from the map — the reaper's reconcile pass treats absence as
    /// "deleted". One indexed query (`task_id` is the primary key).
    async fn get_task_statuses(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, TaskStatus>, sqlx::Error>;

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
    /// `scope_kind` is `"user"`, `"team"`, or `"global"`. `scope_id` is the
    /// user_id or team_id; ignored for global. Pass `""` for `except_task_id`
    /// to disable the exclusion. Owned `String` params keep `mockall::automock`
    /// happy — `Option<&str>` and `&str` elide lifetimes the macro can't
    /// synthesise.
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
    async fn find_by_id_internal(&self, task_id: &str) -> Result<Option<Task>, sqlx::Error> {
        sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE task_id = $1")
            .bind(task_id)
            .fetch_optional(&self.0)
            .await
    }

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
            .bind(&ctx.team_ids)
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
                created_by, visibility, scope, team_id, owner_user_id,
                append_system_prompt, updated_at, headless, ttl_minutes,
                inactive_ttl_days, trigger_id, disabled_servers,
                workspace_id
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21,
                $22, NOW(), $23, $24, $25, $26, $27,
                $28
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
        .bind(&row.team_id)
        .bind(&row.owner_user_id)
        .bind(&row.append_system_prompt)
        .bind(row.headless)
        .bind(row.ttl_minutes)
        .bind(row.inactive_ttl_days)
        .bind(&row.trigger_id)
        .bind(&row.disabled_servers)
        .bind(&workspace_id)
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
        // Use a CTE with window function to get total count in one query
        #[derive(sqlx::FromRow)]
        struct TaskWithCount {
            task_id: String,
            short_id: Option<String>,
            created_at: String,
            last_message_at: String,
            agent_type: AgentType,
            repository_id: String,
            workspace_id: Option<String>,
            status: TaskStatus,
            total_cost_usd: f64,
            site_url: Option<String>,
            template_id: Option<String>,
            vibe: Option<bool>,
            name: Option<String>,
            description: Option<String>,
            site_id: Option<String>,
            site_provider: Option<String>,
            branch: Option<String>,
            created_by: Option<String>,
            visibility: Visibility,
            scope: Scope,
            team_id: Option<String>,
            owner_user_id: Option<String>,
            created_at_db: Option<DateTime<Utc>>,
            updated_at: Option<DateTime<Utc>>,
            append_system_prompt: Option<String>,
            headless: bool,
            ttl_minutes: i32,
            inactive_ttl_days: i32,
            trigger_id: Option<String>,
            disabled_servers: Option<serde_json::Value>,
            total_count: i64,
        }

        let access = filter.scope.access_filter(ScopedEntity::Task, "t.", 1);
        // The parent_task_id filter joins against task_relations rather than
        // reading a column off `tasks`. When unset, the EXISTS clause is a
        // no-op (`$8 IS NULL`).
        let sql = format!(
            r#"SELECT DISTINCT
                t.task_id, t.short_id, t.created_at, t.last_message_at, t.agent_type,
                t.repository_id, t.workspace_id, t.status, t.total_cost_usd, t.site_url, t.template_id,
                t.vibe, t.name, t.description, t.site_id, t.site_provider, t.branch,
                t.created_by, t.visibility, t.scope, t.team_id, t.owner_user_id,
                t.created_at_db, t.updated_at, t.append_system_prompt,
                t.headless, t.ttl_minutes,
                t.inactive_ttl_days, t.trigger_id, t.disabled_servers,
                COUNT(*) OVER() AS total_count
            FROM tasks t
            LEFT JOIN task_tags tt ON t.task_id = tt.task_id
            LEFT JOIN tags tg ON tt.tag_id = tg.tag_id
            WHERE {}
              AND ($4::text IS NULL OR t.status = $4)
              AND ($11::text IS NULL OR t.status != $11)
              AND ($5::text IS NULL OR t.repository_id = $5)
              AND ($6::text IS NULL OR t.created_by = $6)
              AND ($7::text IS NULL OR tg.name = $7)
              AND ($8::text IS NULL OR EXISTS (
                  SELECT 1 FROM task_relations tr
                  WHERE tr.from_task_id = t.task_id
                    AND tr.relation_type = 'created_by'
                    AND tr.to_task_id = $8
              ))
              AND ($12::text IS NULL OR EXISTS (
                  SELECT 1 FROM task_follows tf
                  WHERE tf.task_id = t.task_id
                    AND tf.user_id = $12
              ))
            ORDER BY t.updated_at DESC NULLS LAST
            LIMIT $9 OFFSET $10"#,
            access.sql(),
        );
        let rows = sqlx::query_as::<_, TaskWithCount>(AssertSqlSafe(sql))
            .bind(filter.scope.is_admin)
            .bind(&filter.scope.team_ids)
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

        let tasks: Vec<Task> = rows
            .into_iter()
            .map(|r| Task {
                task_id: r.task_id,
                short_id: r.short_id,
                created_at: r.created_at,
                last_message_at: r.last_message_at,
                agent_type: r.agent_type,
                repository_id: r.repository_id,
                workspace_id: r.workspace_id,
                status: r.status,
                total_cost_usd: r.total_cost_usd,
                site_url: r.site_url,
                template_id: r.template_id,
                vibe: r.vibe,
                name: r.name,
                description: r.description,
                site_id: r.site_id,
                site_provider: r.site_provider,
                branch: r.branch,
                created_by: r.created_by,
                visibility: r.visibility,
                scope: r.scope,
                team_id: r.team_id,
                owner_user_id: r.owner_user_id,
                created_at_db: r.created_at_db,
                updated_at: r.updated_at,
                append_system_prompt: r.append_system_prompt,
                headless: r.headless,
                ttl_minutes: r.ttl_minutes,
                inactive_ttl_days: r.inactive_ttl_days,
                trigger_id: r.trigger_id,
                disabled_servers: r.disabled_servers,
            })
            .collect();

        Ok((tasks, total))
    }

    async fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), sqlx::Error> {
        // Use individual UPDATE statements per field for type safety
        // rather than dynamic SQL construction
        if let Some(ref status) = patch.status {
            sqlx::query("UPDATE tasks SET status = $2, updated_at = NOW() WHERE task_id = $1")
                .bind(task_id)
                .bind(status)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref name) = patch.name {
            sqlx::query("UPDATE tasks SET name = $2, updated_at = NOW() WHERE task_id = $1")
                .bind(task_id)
                .bind(name)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref description) = patch.description {
            sqlx::query("UPDATE tasks SET description = $2, updated_at = NOW() WHERE task_id = $1")
                .bind(task_id)
                .bind(description)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref last_message_at) = patch.last_message_at {
            sqlx::query(
                "UPDATE tasks SET last_message_at = $2, updated_at = NOW() WHERE task_id = $1",
            )
            .bind(task_id)
            .bind(last_message_at)
            .execute(&self.0)
            .await?;
        }
        if let Some(total_cost_usd) = patch.total_cost_usd {
            sqlx::query(
                "UPDATE tasks SET total_cost_usd = $2, updated_at = NOW() WHERE task_id = $1",
            )
            .bind(task_id)
            .bind(total_cost_usd)
            .execute(&self.0)
            .await?;
        }
        if let Some(increment) = patch.total_cost_usd_increment {
            sqlx::query("UPDATE tasks SET total_cost_usd = total_cost_usd + $2, updated_at = NOW() WHERE task_id = $1")
                .bind(task_id)
                .bind(increment)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref site_url) = patch.site_url {
            sqlx::query("UPDATE tasks SET site_url = $2, updated_at = NOW() WHERE task_id = $1")
                .bind(task_id)
                .bind(site_url)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref visibility) = patch.visibility {
            sqlx::query("UPDATE tasks SET visibility = $2, updated_at = NOW() WHERE task_id = $1")
                .bind(task_id)
                .bind(visibility)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref short_id) = patch.short_id {
            sqlx::query("UPDATE tasks SET short_id = $2, updated_at = NOW() WHERE task_id = $1")
                .bind(task_id)
                .bind(short_id)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref append_system_prompt) = patch.append_system_prompt {
            sqlx::query(
                "UPDATE tasks SET append_system_prompt = $2, updated_at = NOW() WHERE task_id = $1",
            )
            .bind(task_id)
            .bind(append_system_prompt)
            .execute(&self.0)
            .await?;
        }
        if let Some(ttl_minutes) = patch.ttl_minutes {
            sqlx::query("UPDATE tasks SET ttl_minutes = $2, updated_at = NOW() WHERE task_id = $1")
                .bind(task_id)
                .bind(ttl_minutes)
                .execute(&self.0)
                .await?;
        }
        if let Some(inactive_ttl_days) = patch.inactive_ttl_days {
            sqlx::query(
                "UPDATE tasks SET inactive_ttl_days = $2, updated_at = NOW() WHERE task_id = $1",
            )
            .bind(task_id)
            .bind(inactive_ttl_days)
            .execute(&self.0)
            .await?;
        }
        if let Some(ref disabled_servers) = patch.disabled_servers {
            // Full replace — the column always holds the complete disabled set.
            let json = serde_json::json!(disabled_servers);
            sqlx::query(
                "UPDATE tasks SET disabled_servers = $2, updated_at = NOW() WHERE task_id = $1",
            )
            .bind(task_id)
            .bind(json)
            .execute(&self.0)
            .await?;
        }
        Ok(())
    }

    async fn update_status_if_not_closed(
        &self,
        task_id: &str,
        new_status: TaskStatus,
    ) -> Result<bool, sqlx::Error> {
        let res = sqlx::query(
            "UPDATE tasks SET status = $2, updated_at = NOW() \
             WHERE task_id = $1 AND status <> 'closed'",
        )
        .bind(task_id)
        .bind(new_status)
        .execute(&self.0)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn delete_task(&self, task_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM tasks WHERE task_id = $1")
            .bind(task_id)
            .execute(&self.0)
            .await?;
        Ok(())
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
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(
            String,
            Option<DateTime<Utc>>,
            SandboxDesiredState,
            Option<SandboxStopReason>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            r#"SELECT task_id, started_at, desired_state, stop_reason, desired_state_updated_at
               FROM sandboxes
               WHERE task_id = ANY($1)"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(task_id, started_at, desired_state, stop_reason, desired_state_updated_at)| {
                    (
                        task_id,
                        SandboxIntentView {
                            started_at,
                            desired_state,
                            stop_reason,
                            desired_state_updated_at,
                        },
                    )
                },
            )
            .collect())
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

    async fn get_task_statuses(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, TaskStatus>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, TaskStatus)> =
            sqlx::query_as(r#"SELECT task_id, status FROM tasks WHERE task_id = ANY($1)"#)
                .bind(task_ids)
                .fetch_all(&self.0)
                .await?;

        Ok(rows.into_iter().collect())
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
            "team" => {
                sqlx::query_as(
                    r#"SELECT DISTINCT t.task_id FROM tasks t
                       JOIN team_members tm ON tm.user_id::text = t.created_by
                       WHERE t.status = 'open'
                         AND t.agent_type = 'codex'
                         AND tm.team_id = $1
                         AND ($2::text IS NULL OR t.task_id <> $2)"#,
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

/// Find by task_id without access control (convenience wrapper).
pub async fn find_by_id_internal(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<Task>, sqlx::Error> {
    PgTaskRepository(pool.clone())
        .find_by_id_internal(task_id)
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
) -> Result<(), sqlx::Error> {
    PgTaskRepository(pool.clone())
        .update_task(task_id, patch)
        .await
}

/// Delete a task (convenience wrapper).
pub async fn delete_task(pool: &DbPool, task_id: &str) -> Result<(), sqlx::Error> {
    PgTaskRepository(pool.clone()).delete_task(task_id).await
}

/// Check if a task with the given short_id exists (convenience wrapper).
pub async fn exists_by_short_id(pool: &DbPool, short_id: &str) -> Result<bool, sqlx::Error> {
    PgTaskRepository(pool.clone())
        .exists_by_short_id(short_id)
        .await
}
