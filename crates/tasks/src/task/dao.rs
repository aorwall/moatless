//! Task DAO — queries for task data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use super::models::{
    CheckoutMode, SandboxDesiredState, SandboxIntentView, SandboxPlacement, Task, TaskFilter,
    TaskPatch, TaskStatus, Visibility, sandbox_intent_epoch,
};
use super::pin_dao::{self, OrderSlot};
use super::settle_dao;
use super::snooze_dao;
use moatless_core::db::DbPool;
use moatless_core::db_enum::decode_optional;
use moatless_types::access_control::{Scope, ScopeContext, ScopedEntity};
use moatless_types::agent::AgentProfile;
use moatless_types::task::TaskRelationType;

#[derive(sqlx::FromRow)]
struct SandboxIntentViewRow {
    task_id: String,
    started_at: Option<DateTime<Utc>>,
    desired_state: Option<String>,
    stop_reason: Option<String>,
    desired_state_updated_at: Option<DateTime<Utc>>,
}

/// Batch-load the task-response projection of Sandbox intent. A Task with no
/// Sandbox row returns converged Stopped at the no-intent epoch; `Lifecycle`
/// remains the sole *writer*.
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
           LEFT JOIN sandboxes s ON s.sandbox_id = t.sandbox_id
           WHERE t.task_id = ANY($1)"#,
    )
    .bind(task_ids)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok((
                row.task_id,
                SandboxIntentView {
                    started_at: row.started_at,
                    desired_state: decode_optional("desired_state", row.desired_state)?
                        .unwrap_or(SandboxDesiredState::Stopped),
                    stop_reason: decode_optional("stop_reason", row.stop_reason)?,
                    desired_state_updated_at: row
                        .desired_state_updated_at
                        .unwrap_or_else(sandbox_intent_epoch),
                },
            ))
        })
        .collect()
}

/// DAO trait for task persistence. The generated mock inherits this trait's
/// visibility; `test-support` gates it since
/// [`super::test_support::ScriptedTasks`] uses the mock in a host test.
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait]
pub(crate) trait TaskDao: Send + Sync {
    /// Find by `task_id`, scoped to access control.
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error>;
    /// The status of the Task [`Self::find_by_id`] would return, without
    /// reading the rest of the row.
    async fn find_readable_status(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskStatus>, sqlx::Error>;
    /// Find by `task_id`, scoped to the `write_filter` rule.
    async fn find_written_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error>;
    /// Find by `task_id`, scoped to the `owner_filter` rule.
    async fn find_owned_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error>;
    /// Insert a task. `row.workspace_id` must be set: every Task runs in a
    /// Workspace, and the Sandbox row written alongside it is keyed on that id.
    async fn insert_task(
        &self,
        row: &Task,
        placement: &SandboxPlacement,
    ) -> Result<(), sqlx::Error>;
    /// Undo an `insert_task` whose creation then failed: removes the
    /// just-created Task and its rows, never the Sandbox row — it belongs to
    /// the members the Task tried to join.
    async fn delete_inserted_task(&self, task_id: &str) -> Result<bool, sqlx::Error>;
    /// Free the id of a Task that errored before anything was said, reporting
    /// whether one existed. The `Error`/no-messages/`ctx`-owned check runs
    /// atomically in `WHERE`; deletes all rows under the id.
    async fn delete_errored_empty_task(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;
    async fn list_tasks(&self, filter: &TaskFilter) -> Result<(Vec<Task>, i64), sqlx::Error>;
    /// Update task fields for the caller's scope, one statement per field, each
    /// embedding `write_filter`, or `owner_filter` for a
    /// [`TaskPatch::owner_only`] patch. An empty `patch` returns `Ok(true)` with no writes
    /// — the caller already read-checked.
    async fn update_task(
        &self,
        task_id: &str,
        patch: &TaskPatch,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;
    /// Recompute `tasks.total_cost_usd` as the sum of every turn's priced cost,
    /// from durable `task_turns.metrics` rows — idempotent, since a redelivered
    /// event yields the same sum, unlike an increment.
    async fn recompute_total_cost(&self, task_id: &str) -> Result<(), sqlx::Error>;
    /// Flip status to `new_status` unless already `Closed`, atomically via
    /// `WHERE status <> 'closed'` so no racing event resurrects a `Closed`
    /// task. System callers pass `ScopeContext::system()`.
    async fn update_status_if_not_closed(
        &self,
        task_id: &str,
        new_status: TaskStatus,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;
    /// Delete a task. Returns `true` when a row was deleted, `false` when the
    /// task was not found or the caller does not own it (existence hiding).
    async fn delete_task(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;

    /// Claim an unused Task for deletion; inherited conversation does not count as use.
    async fn close_if_unused(&self, task_id: &str, ctx: &ScopeContext)
    -> Result<bool, sqlx::Error>;

    /// Side chats owned by this parent, including closed ones.
    async fn side_chat_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error>;
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
    /// reason, timestamps) from `sandboxes`. A Task with no Sandbox row gets
    /// the lazy converged-Stopped default.
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
    /// Batch-fetch `close_after_settled_days` for the given tasks. Tasks with no row
    /// are omitted (not present in the map).
    async fn get_close_after_settled_days_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error>;
    /// Batch-fetch `settle_after_hours` for the given tasks. Like
    /// `get_cold_after_hours_map`, an absent entry means "deployment default".
    async fn get_settle_after_hours_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error>;
    /// When each of the given tasks became settled, for those that are: every
    /// follower has settled it, or nobody follows it and the reaper settled it.
    /// The latest settle time of the task and its followers wins.
    async fn get_settled_since_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;
    /// Settle `task_id` for every follower who has not pinned it, unsettled it
    /// by hand, opted it out, or had it asleep since `idle_since`, and stamp
    /// the task's own `auto_settled_at`. Returns whether any row moved.
    async fn auto_settle_task(
        &self,
        task_id: &str,
        at: DateTime<Utc>,
        idle_since: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error>;
    /// Batch-fetch `cold_after_hours` for the given tasks. The column is
    /// nullable and tasks that leave it unset are omitted, so an absent entry
    /// and a `NULL` both mean "follow the deployment default".
    async fn get_cold_after_hours_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error>;
    /// List task IDs of all tasks currently in `status = 'open'`.
    async fn list_open_task_ids(&self) -> Result<Vec<String>, sqlx::Error>;

    /// List `(task_id, sandbox_id)` for every active member of the given
    /// sandboxes. A member is a task attached to the sandbox, including the one
    /// whose id names it, so a sandbox with no fork returns one pair.
    async fn list_sandbox_members(
        &self,
        sandbox_ids: &[String],
    ) -> Result<Vec<(String, String)>, sqlx::Error>;

    /// The Sandbox to stop now that `task_id`'s agent run has finished, when it
    /// asked to be stopped that way (`sandboxes.stop_when_done`) and nothing
    /// else is still using it.
    ///
    /// `None` on every other shape — still busy, or a turn owed — leaves the
    /// pod to the idle reaper, the only other thing that stops it.
    async fn sandbox_to_stop_when_done(&self, task_id: &str)
    -> Result<Option<String>, sqlx::Error>;

    /// Upsert a user's read marker for a task. Monotonic — never moves the
    /// stored `last_read_at` backwards, so out-of-order calls are safe.
    async fn mark_task_read(
        &self,
        user_id: &str,
        task_id: &str,
        read_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    /// Rewind a user's read marker to just before `task_id`'s latest turn
    /// ended, so that turn reads as unseen; removes the marker when no turn
    /// has ended.
    async fn mark_task_unread(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error>;

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

    /// Make `user_id` stop following `task_id`. Idempotent — unfollowing a task
    /// that isn't followed is a no-op.
    async fn unfollow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error>;

    async fn settle_task(
        &self,
        user_id: &str,
        task_id: &str,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    async fn unsettle_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error>;

    /// Let the reaper auto-settle `task_id` for `user_id` again, or opt it out
    /// from `at`. Settles or unsettles nothing itself.
    async fn set_auto_settle(
        &self,
        user_id: &str,
        task_id: &str,
        enabled: bool,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    async fn snooze_task(
        &self,
        user_id: &str,
        task_id: &str,
        until: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    /// Clear the snooze, stamping `snooze_woke_at` with when it woke: `at`, or
    /// its own timer when that ran out first.
    async fn unsnooze_task(
        &self,
        user_id: &str,
        task_id: &str,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    /// Pin `task_id` in `user_id`'s listing. Pinning promotes: it also
    /// un-settles the Task and clears its snooze. `order_key` is taken only
    /// by a fresh pin, so a repeated pin keeps the slot the user arranged.
    async fn pin_task(
        &self,
        user_id: &str,
        task_id: &str,
        order_key: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    /// Unpin, dropping the pinned slot with the pin.
    async fn unpin_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error>;

    /// Set the fractional-index key `user_id`'s `slot` block sorts by.
    async fn set_order_key(
        &self,
        user_id: &str,
        task_id: &str,
        slot: OrderSlot,
        order_key: &str,
    ) -> Result<(), sqlx::Error>;

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
}

/// PostgreSQL implementation of [`TaskDao`].
pub(crate) struct PgTaskDao {
    pub(super) pool: DbPool,
}

impl PgTaskDao {
    pub(crate) fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TaskDao for PgTaskDao {
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error> {
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "", 2);
        self.find_under(task_id, ctx, &access).await
    }

    async fn find_readable_status(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskStatus>, sqlx::Error> {
        self.status_under(task_id, ctx).await
    }

    async fn find_written_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error> {
        let write = moatless_core::db::access_control::write_filter(ScopedEntity::Task, "", 2);
        self.find_under(task_id, ctx, &write).await
    }

    async fn find_owned_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error> {
        let owner = moatless_core::db::access_control::owner_filter("", 2);
        self.find_under(task_id, ctx, &owner).await
    }

    async fn insert_task(
        &self,
        row: &Task,
        placement: &SandboxPlacement,
    ) -> Result<(), sqlx::Error> {
        // The column is nullable for rows written before workspaces existed, so
        // the invariant is enforced here rather than by the schema.
        let workspace_id = row.workspace_id.clone().ok_or_else(|| {
            sqlx::Error::Protocol(format!(
                "task {} has no workspace_id; every task is created in a workspace",
                row.task_id
            ))
        })?;

        let mut tx = self.pool.begin().await?;

        // Task controls Sandbox: naming no sandbox writes the Sandbox row
        // here in the same transaction as the Task; naming one joins the
        // existing Sandbox instead. `run_as_user_id` follows `Actor::run_as`
        // (Task > Workspace account > work owner). `storage_reclaimed_at` is
        // stamped since a never-started Sandbox owns no volume.
        let sandbox_id = row
            .sandbox_id
            .clone()
            .unwrap_or_else(|| row.task_id.clone());
        if row.sandbox_id.is_none() {
            let owner = row.owner_user_id.as_deref().or(row.created_by.as_deref());
            let inserted = sqlx::query(
                r#"INSERT INTO sandboxes (
                       sandbox_id, workspace_id, scope, owner_user_id, run_as_user_id,
                       branch, headless, agent_profile, use_tmux, storage_reclaimed_at,
                       node_selector, stop_when_done
                   )
                   SELECT $1, w.id, $2, $3,
                          COALESCE(NULLIF($9, ''), NULLIF(w.run_as_user_id, ''), $3),
                          $4, $5, $6, $7, NOW(),
                          COALESCE($10, w.node_selector), $11
                   FROM workspaces w
                   WHERE w.id = $8"#,
            )
            .bind(&sandbox_id)
            .bind(row.scope.as_str())
            .bind(owner)
            .bind(&row.branch)
            .bind(row.headless)
            .bind(row.agent_profile.as_str())
            // The catalog is the authority for which harness runs over tmux, and
            // it lives in this band. The control plane reads the answer.
            .bind(crate::agent::catalog::use_tmux_for(row.agent_profile))
            .bind(&workspace_id)
            .bind(&row.run_as_user_id)
            .bind(placement.node_selector.as_ref().map(sqlx::types::Json))
            .bind(placement.stop_when_done)
            .execute(&mut *tx)
            .await?;
            if inserted.rows_affected() == 0 {
                return Err(sqlx::Error::Protocol(format!(
                    "workspace {workspace_id} does not exist; \
                     cannot create sandbox {sandbox_id} for task {}",
                    row.task_id
                )));
            }
        }

        sqlx::query(
            r#"INSERT INTO tasks (
                task_id, short_id, created_at, last_message_at, agent_profile,
                repository_id, status, total_cost_usd, site_url, template_id,
                vibe, name, description, site_id, site_provider, branch,
                created_by, visibility, scope, owner_user_id, run_as_user_id,
                append_system_prompt, updated_at, headless, ttl_minutes,
                close_after_settled_days, loop_id, model,
                workspace_id, sandbox_id, checkout_mode, skills, memory_preload,
                settle_after_hours
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                $21, $22, NOW(), $23, $24, $25, $26,
                $27, $28, $29, $30, $31, $32, $33
            )"#,
        )
        .bind(&row.task_id)
        .bind(&row.short_id)
        .bind(row.created_at.to_rfc3339())
        .bind(row.last_message_at.to_rfc3339())
        .bind(row.agent_profile.as_str())
        .bind(&row.repository_id)
        .bind(row.status.as_str())
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
        .bind(row.visibility.as_str())
        .bind(row.scope.as_str())
        .bind(&row.owner_user_id)
        .bind(&row.run_as_user_id)
        .bind(&row.append_system_prompt)
        .bind(row.headless)
        .bind(row.ttl_minutes)
        .bind(row.close_after_settled_days)
        .bind(&row.loop_id)
        .bind(&row.model)
        .bind(&workspace_id)
        .bind(&sandbox_id)
        .bind(row.checkout_mode.map(CheckoutMode::as_str))
        .bind(&row.skills)
        .bind(&row.memory_preload)
        .bind(row.settle_after_hours)
        .execute(&mut *tx)
        .await?;

        // A conversation coming into existence is a listing change. Last, after
        // the `tasks` insert — `record_change`'s "call this last".
        moatless_core::change_feed::listing::record_change(&mut tx, &row.task_id).await?;
        tx.commit().await?;

        // Auto-follow the creator so every creation path (HTTP, adapters,
        // loops) behaves consistently; best-effort, never fails the insert.
        // Deliberately after the commit, not inside it: an error inside the
        // tx would abort it. It records its own change, coalescing with the
        // task's into one delivered row.
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

    async fn delete_inserted_task(&self, task_id: &str) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM task_tags WHERE task_id = $1")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM task_follows WHERE task_id = $1")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM task_read_receipts WHERE task_id = $1")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;

        let parents = super::relations_dao::parent_ids(&mut tx, task_id).await?;
        let deleted: Option<String> =
            sqlx::query_scalar("DELETE FROM tasks WHERE task_id = $1 RETURNING task_id")
                .bind(task_id)
                .fetch_optional(&mut *tx)
                .await?;
        let did_delete = deleted.is_some();
        if did_delete {
            crate::conversation::dao::delete_task(&mut tx, task_id).await?;
            record_deletion(&mut tx, task_id, &parents).await?;
        }
        tx.commit().await?;
        Ok(did_delete)
    }

    async fn delete_errored_empty_task(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        // `status`, the empty conversation and the owner rule are all in this
        // one `WHERE`, so a Task that became usable between the caller's read
        // and this call is not deleted by a decision that has gone stale.
        let ctrl = moatless_core::db::access_control::owner_filter("", 2);
        let sql = format!(
            "DELETE FROM tasks \
             WHERE task_id = $1 AND status = $4 AND {} \
               AND NOT EXISTS (SELECT 1 FROM conversation_messages WHERE task_id = $1) \
             RETURNING task_id, sandbox_id",
            ctrl.sql()
        );
        let mut tx = self.pool.begin().await?;
        let parents = super::relations_dao::parent_ids(&mut tx, task_id).await?;
        let deleted: Option<(String, Option<String>)> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .bind(TaskStatus::Error.as_str())
            .fetch_optional(&mut *tx)
            .await?;
        let Some((deleted_task_id, sandbox_id)) = deleted else {
            tx.commit().await?;
            return Ok(false);
        };

        // The Sandbox this Task was created with — never one it joined, since a
        // join failure deletes the Task instead of leaving it errored. Guarded
        // by the same last-member rule as `delete_task` anyway, so a shared
        // Sandbox reached by some other route keeps its row.
        if let Some(sandbox_id) = sandbox_id {
            let sandbox_exists: Option<String> = sqlx::query_scalar(
                "SELECT sandbox_id FROM sandboxes WHERE sandbox_id = $1 FOR UPDATE",
            )
            .bind(&sandbox_id)
            .fetch_optional(&mut *tx)
            .await?;
            if sandbox_exists.is_some() {
                let has_remaining_members: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks WHERE sandbox_id = $1)")
                        .bind(&sandbox_id)
                        .fetch_one(&mut *tx)
                        .await?;
                if !has_remaining_members {
                    sqlx::query("DELETE FROM sandboxes WHERE sandbox_id = $1")
                        .bind(&sandbox_id)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }

        // The failed incarnation's trail. Its activities are the part that
        // would be visible: a preflight failure publishes a Sandbox error
        // event, which is projected as an activity, and the conversation the
        // retry opens is read from that table.
        crate::conversation::dao::delete_task(&mut tx, &deleted_task_id).await?;
        sqlx::query("DELETE FROM task_events WHERE task_id = $1")
            .bind(&deleted_task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM task_tags WHERE task_id = $1")
            .bind(&deleted_task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM task_follows WHERE task_id = $1")
            .bind(&deleted_task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM task_read_receipts WHERE task_id = $1")
            .bind(&deleted_task_id)
            .execute(&mut *tx)
            .await?;

        record_deletion(&mut tx, &deleted_task_id, &parents).await?;
        tx.commit().await?;
        Ok(true)
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
            agent_profile: Option<String>,
            repository_id: Option<String>,
            workspace_id: Option<String>,
            sandbox_id: Option<String>,
            checkout_mode: Option<String>,
            status: Option<String>,
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
            visibility: Option<String>,
            scope: Option<String>,
            owner_user_id: Option<String>,
            run_as_user_id: Option<String>,
            updated_at: Option<DateTime<Utc>>,
            append_system_prompt: Option<String>,
            headless: Option<bool>,
            ttl_minutes: Option<i32>,
            close_after_settled_days: Option<i32>,
            settle_after_hours: Option<i32>,
            loop_id: Option<String>,
            skills: Option<Vec<String>>,
            memory_preload: Option<Vec<String>>,
        }

        impl TaskPageRow {
            /// The total-only row the query emits when the page is empty, which
            /// carries the total but no task.
            fn is_total_only(&self) -> bool {
                self.task_id.is_none()
            }

            /// `None` only when a `NOT NULL` column came back NULL, or held an
            /// unnamed value. Callers must reject that, not skip the row —
            /// skipping would drift the page's tasks from its total.
            fn into_task(self) -> Option<Task> {
                fn timestamp(value: &str) -> Option<DateTime<Utc>> {
                    DateTime::parse_from_rfc3339(value)
                        .ok()
                        .map(|parsed| parsed.with_timezone(&Utc))
                }

                Some(Task {
                    task_id: self.task_id?,
                    short_id: self.short_id,
                    created_at: timestamp(&self.created_at?)?,
                    last_message_at: timestamp(&self.last_message_at?)?,
                    agent_profile: AgentProfile::parse(&self.agent_profile?)?,
                    repository_id: self.repository_id,
                    workspace_id: self.workspace_id,
                    sandbox_id: self.sandbox_id,
                    checkout_mode: self
                        .checkout_mode
                        .and_then(|v| CheckoutMode::try_from(v).ok()),
                    status: TaskStatus::try_from(self.status?.as_str()).ok()?,
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
                    visibility: Visibility::try_from(self.visibility?.as_str()).ok()?,
                    scope: Scope::try_from(self.scope?).ok()?,
                    owner_user_id: self.owner_user_id,
                    run_as_user_id: self.run_as_user_id,
                    updated_at: self.updated_at,
                    append_system_prompt: self.append_system_prompt,
                    headless: self.headless?,
                    ttl_minutes: self.ttl_minutes?,
                    close_after_settled_days: self.close_after_settled_days?,
                    settle_after_hours: self.settle_after_hours,
                    loop_id: self.loop_id,
                    // NOT NULL in `tasks`, so only the total-only row can be
                    // NULL here — and that row is rejected before this point.
                    skills: self.skills.unwrap_or_default(),
                    memory_preload: self.memory_preload.unwrap_or_default(),
                })
            }
        }

        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "t.", 1);
        // `filtered` is the single source for page and total, so they can't
        // drift apart. `EXISTS` subqueries (not joins) keep tag/relation
        // filters from fanning a multi-tag task into several rows and
        // over-counting. `total LEFT JOIN page ON TRUE` keeps the total
        // available when an offset past the end leaves the page empty.
        let sql = format!(
            r#"WITH filtered AS (
                SELECT
                    t.task_id, t.short_id, t.created_at, t.last_message_at, t.agent_profile,
                    t.repository_id, t.workspace_id, t.status, t.total_cost_usd, t.site_url, t.template_id,
                    t.sandbox_id, t.checkout_mode,
                    t.vibe, t.name, t.description, t.site_id, t.site_provider, t.branch,
                    t.model, t.created_by, t.visibility, t.scope, t.owner_user_id,
                    t.run_as_user_id,
                    t.created_at_db, t.updated_at, t.append_system_prompt,
                    t.headless, t.ttl_minutes,
                    t.close_after_settled_days, t.settle_after_hours, t.loop_id, t.skills,
                    t.memory_preload
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
                  -- Both parent edges: a fork shows up under the Task it was
                  -- branched off, exactly as a child shows up under its creator.
                  AND ($7::text IS NULL OR EXISTS (
                      SELECT 1 FROM task_relations tr
                      WHERE tr.from_task_id = t.task_id
                        AND tr.relation_type = ANY($12)
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
            .bind(filter.status.map(TaskStatus::as_str))
            .bind(&filter.repository_id)
            .bind(&filter.created_by)
            .bind(&filter.tag)
            .bind(&filter.parent_task_id)
            .bind(filter.limit)
            .bind(filter.offset)
            .bind(filter.exclude_status.map(TaskStatus::as_str))
            .bind(&filter.followed_by)
            .bind(&TaskRelationType::PARENT_WIRE[..])
            .fetch_all(&self.pool)
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
            let task_id = row.task_id.clone().unwrap_or_default();
            tasks.push(row.into_task().ok_or_else(|| {
                sqlx::Error::Decode(
                    format!(
                        "task row `{task_id}` has a NULL or unknown value in a NOT NULL column"
                    )
                    .into(),
                )
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
        // Per-field UPDATE statements (type safety), each scoped by one filter:
        // `owner_filter` when the patch carries an owner-only field, so a
        // mixed patch cannot land its other fields without it.
        // `first_affected` takes the first non-empty field's `rows_affected`
        // — valid since every statement shares one `WHERE`. One transaction:
        // the listing change and mutation must commit together or not at all.
        let mut tx = self.pool.begin().await?;
        let ctrl = if patch.owner_only() {
            moatless_core::db::access_control::owner_filter("", 3)
        } else {
            moatless_core::db::access_control::write_filter(ScopedEntity::Task, "", 3)
        };
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
                status.as_str()
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
        if let Some(ref site_url) = patch.site_url {
            run_field!("site_url = $2", site_url);
        }
        if let Some(ref visibility) = patch.visibility {
            run_field!("visibility = $2", visibility.as_str());
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
        if let Some(close_after_settled_days) = patch.close_after_settled_days {
            run_field!("close_after_settled_days = $2", close_after_settled_days);
        }
        // Last, after every `tasks` statement above — `record_change`'s "call
        // this last". Gated twice: on `rows_affected`, because a patch the
        // caller may not control moves nothing; and on `touches_listing`,
        // because most of this patch is not sidebar-visible and the busiest of
        // those paths runs once per turn.
        if patch.touches_listing() && first_affected.is_some_and(|n| n > 0) {
            moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;

        // Empty patch: caller read-checked the task first; return true.
        Ok(first_affected.is_none_or(|n| n > 0))
    }

    async fn recompute_total_cost(&self, task_id: &str) -> Result<(), sqlx::Error> {
        // Only turns whose `totalCostUsd` is a JSON number contribute; a turn
        // that never priced (missing key, JSON null) is skipped, not summed as
        // zero-or-error. `COALESCE` keeps a task with no priced turns at 0.
        sqlx::query(
            "UPDATE tasks SET total_cost_usd = COALESCE((\
                 SELECT SUM((metrics->>'totalCostUsd')::float8) \
                 FROM task_turns \
                 WHERE task_id = $1 AND jsonb_typeof(metrics->'totalCostUsd') = 'number'\
             ), 0), updated_at = NOW() \
             WHERE task_id = $1",
        )
        .bind(task_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_status_if_not_closed(
        &self,
        task_id: &str,
        new_status: TaskStatus,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        let ctrl = moatless_core::db::access_control::write_filter(ScopedEntity::Task, "", 3);
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
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(new_status.as_str())
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&mut *tx)
            .await?;
        // Only when the guard let the flip through: a task already `closed`
        // matches nothing here, and a no-op must not advance the position.
        // Last, after the `tasks` row lock — `record_change`'s "call this last".
        if res.rows_affected() > 0 {
            moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    async fn close_if_unused(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        let ctrl = moatless_core::db::access_control::owner_filter("", 2);
        let sql = format!(
            "UPDATE tasks SET status = 'closed' WHERE task_id = $1 AND {} \
             AND NOT EXISTS (SELECT 1 FROM task_inbox WHERE task_id = $1) \
             AND NOT EXISTS (SELECT 1 FROM task_turns WHERE task_id = $1 AND turn_number > COALESCE((SELECT at_turn FROM task_forks WHERE task_id = $1), 0)) \
             AND NOT EXISTS (SELECT 1 FROM trajectory_sessions s JOIN trajectory_events e ON e.session_id = s.id WHERE s.task_id = $1) \
             AND NOT EXISTS (SELECT 1 FROM conversation_messages m LEFT JOIN task_events e ON e.event_id = m.task_event_id WHERE m.task_id = $1 AND (m.frozen OR e.task_id = $1))",
            ctrl.sql()
        );
        Ok(sqlx::query(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            > 0)
    }

    async fn side_chat_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT from_task_id FROM task_relations WHERE to_task_id = $1 AND relation_type = 'forked_from' AND context -> 'side_chat' = 'true'::jsonb")
            .bind(task_id).fetch_all(&self.pool).await
    }

    async fn delete_task(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let ctrl = moatless_core::db::access_control::owner_filter("", 2);
        let sql = format!(
            "DELETE FROM tasks WHERE task_id = $1 AND {} RETURNING task_id, sandbox_id",
            ctrl.sql()
        );
        let mut tx = self.pool.begin().await?;
        let parents = super::relations_dao::parent_ids(&mut tx, task_id).await?;
        let deleted: Option<(String, Option<String>)> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        let did_delete = deleted.is_some();
        if let Some((deleted_task_id, sandbox_id)) = deleted {
            crate::conversation::dao::delete_task(&mut tx, &deleted_task_id).await?;
            // No foreign key cascades these per-person rows.
            sqlx::query("DELETE FROM task_tags WHERE task_id = $1")
                .bind(&deleted_task_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM task_follows WHERE task_id = $1")
                .bind(&deleted_task_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM task_read_receipts WHERE task_id = $1")
                .bind(&deleted_task_id)
                .execute(&mut *tx)
                .await?;
            if let Some(sandbox_id) = sandbox_id {
                let sandbox_exists: Option<String> = sqlx::query_scalar(
                    "SELECT sandbox_id FROM sandboxes WHERE sandbox_id = $1 FOR UPDATE",
                )
                .bind(&sandbox_id)
                .fetch_optional(&mut *tx)
                .await?;
                if sandbox_exists.is_some() {
                    let has_remaining_members: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM tasks WHERE sandbox_id = $1)",
                    )
                    .bind(&sandbox_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    if !has_remaining_members {
                        sqlx::query("DELETE FROM sandboxes WHERE sandbox_id = $1")
                            .bind(&sandbox_id)
                            .execute(&mut *tx)
                            .await?;
                    }
                }
            }

            // The record outlives the Task it names — `task_listing_changes`
            // has no foreign key so a deletion stays replayable: a watching
            // client re-reads this id, finds nothing, and drops its row.
            record_deletion(&mut tx, &deleted_task_id, &parents).await?;
        }
        tx.commit().await?;
        Ok(did_delete)
    }

    async fn exists_by_short_id(&self, short_id: &str) -> Result<bool, sqlx::Error> {
        let row: (bool,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM tasks WHERE short_id = $1)")
            .bind(short_id)
            .fetch_one(&self.pool)
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
        .fetch_all(&self.pool)
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
            r#"SELECT t.task_id, s.started_at
               FROM tasks t
               JOIN sandboxes s ON s.sandbox_id = t.sandbox_id
               WHERE t.task_id = ANY($1) AND s.started_at IS NOT NULL"#,
        )
        .bind(task_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn get_sandbox_intents(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, SandboxIntentView>, sqlx::Error> {
        load_sandbox_intent_views(&self.pool, task_ids).await
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
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.into_iter().collect())
    }

    async fn get_close_after_settled_days_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, i32)> = sqlx::query_as(
            r#"SELECT task_id, close_after_settled_days FROM tasks WHERE task_id = ANY($1)"#,
        )
        .bind(task_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn get_cold_after_hours_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // `IS NOT NULL` rather than mapping to a sentinel: an unset column is
        // the same answer as an absent task, and both mean "deployment default".
        let rows: Vec<(String, i32)> = sqlx::query_as(
            r#"SELECT task_id, cold_after_hours FROM tasks
               WHERE task_id = ANY($1) AND cold_after_hours IS NOT NULL"#,
        )
        .bind(task_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn get_settle_after_hours_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, i32)> = sqlx::query_as(
            r#"SELECT task_id, settle_after_hours FROM tasks
               WHERE task_id = ANY($1) AND settle_after_hours IS NOT NULL"#,
        )
        .bind(task_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn get_settled_since_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        settle_dao::settled_since(&self.pool, task_ids).await
    }

    async fn auto_settle_task(
        &self,
        task_id: &str,
        at: DateTime<Utc>,
        idle_since: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        settle_dao::auto_settle(&self.pool, task_id, at, idle_since).await
    }

    async fn list_open_task_ids(&self) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> =
            sqlx::query_as(r#"SELECT task_id FROM tasks WHERE status = 'open'"#)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn list_sandbox_members(
        &self,
        sandbox_ids: &[String],
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        // `error` joins `open` because a member whose last turn failed still
        // holds its checkout and its registered commands.
        sqlx::query_as(
            r#"
            SELECT task_id, sandbox_id
            FROM tasks
            WHERE sandbox_id = ANY($1)
              AND status IN ('open', 'error')
            "#,
        )
        .bind(sandbox_ids)
        .fetch_all(&self.pool)
        .await
    }

    async fn sandbox_to_stop_when_done(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        // Three busy axes, read in one statement so none opens between
        // checks: `running_commands` (idle reaper defers), an `active` turn
        // (redelivered completion racing a newer run), and a user message
        // past the last turn's `ended_at` (owed, not yet opened — see
        // `moatless_types::running::turn_is_owed`).
        sqlx::query_scalar(
            r#"
            SELECT s.sandbox_id
            FROM tasks t
            JOIN sandboxes s ON s.sandbox_id = t.sandbox_id
            WHERE t.task_id = $1
              AND s.stop_when_done
              AND s.running_commands = 0
              AND NOT EXISTS (
                    SELECT 1
                    FROM task_turns tt
                    WHERE tt.task_id = t.task_id
                      AND tt.status IN ('active', 'interrupted')
              )
              AND NOT EXISTS (
                    SELECT 1
                    FROM task_inbox m
                    WHERE m.task_id = t.task_id
                      AND m.status IN ('pending', 'delivered')
                      AND (
                            NOT EXISTS (
                                  SELECT 1
                                  FROM task_turns tt
                                  WHERE tt.task_id = t.task_id
                            )
                            OR m.created_at > (
                                  SELECT tt.ended_at
                                  FROM task_turns tt
                                  WHERE tt.task_id = t.task_id
                                  ORDER BY tt.turn_number DESC
                                  LIMIT 1
                            )
                      )
              )
              AND NOT EXISTS (
                    SELECT 1
                    FROM tasks peer
                    WHERE peer.sandbox_id = s.sandbox_id
                      AND peer.task_id <> t.task_id
                      AND peer.status IN ('open', 'error')
              )
            "#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
    }

    async fn mark_task_read(
        &self,
        user_id: &str,
        task_id: &str,
        read_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let moved = sqlx::query(
            r#"INSERT INTO task_read_receipts (user_id, task_id, last_read_at)
               VALUES ($1, $2, $3)
               ON CONFLICT (user_id, task_id)
               DO UPDATE SET last_read_at = EXCLUDED.last_read_at
               WHERE task_read_receipts.last_read_at < EXCLUDED.last_read_at"#,
        )
        .bind(user_id)
        .bind(task_id)
        .bind(read_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // The T3 shell carries the marker as `lastVisitedAt`.
        if moved > 0 {
            moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn mark_task_unread(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let ended: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT max(ended_at) FROM task_turns WHERE task_id = $1")
                .bind(task_id)
                .fetch_one(&mut *tx)
                .await?;
        let changed = match ended {
            Some(ended) => sqlx::query(
                r#"INSERT INTO task_read_receipts (user_id, task_id, last_read_at)
                   VALUES ($1, $2, $3)
                   ON CONFLICT (user_id, task_id)
                   DO UPDATE SET last_read_at = EXCLUDED.last_read_at
                   WHERE task_read_receipts.last_read_at > EXCLUDED.last_read_at"#,
            )
            .bind(user_id)
            .bind(task_id)
            .bind(ended - chrono::Duration::milliseconds(1))
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            None => {
                sqlx::query("DELETE FROM task_read_receipts WHERE user_id = $1 AND task_id = $2")
                    .bind(user_id)
                    .bind(task_id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
            }
        };
        if changed > 0 {
            moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
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
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
    }

    async fn follow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
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
            moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn unfollow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(r#"DELETE FROM task_follows WHERE user_id = $1 AND task_id = $2"#)
            .bind(user_id)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        // The row leaves this person's listing and nobody else's. The delivery
        // path re-reads it per viewer, so the one whose follow this was finds
        // nothing and the rest see no change at all.
        if res.rows_affected() > 0 {
            moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
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
        settle_dao::settle(&self.pool, user_id, task_id, at).await
    }

    async fn unsettle_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        settle_dao::unsettle(&self.pool, user_id, task_id).await
    }

    async fn set_auto_settle(
        &self,
        user_id: &str,
        task_id: &str,
        enabled: bool,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        settle_dao::set_auto_settle(&self.pool, user_id, task_id, enabled, at).await
    }

    async fn snooze_task(
        &self,
        user_id: &str,
        task_id: &str,
        until: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        snooze_dao::snooze(&self.pool, user_id, task_id, until, at).await
    }

    async fn unsnooze_task(
        &self,
        user_id: &str,
        task_id: &str,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        snooze_dao::unsnooze(&self.pool, user_id, task_id, at).await
    }

    async fn pin_task(
        &self,
        user_id: &str,
        task_id: &str,
        order_key: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        pin_dao::pin(&self.pool, user_id, task_id, order_key, at).await
    }

    async fn unpin_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        pin_dao::unpin(&self.pool, user_id, task_id).await
    }

    async fn set_order_key(
        &self,
        user_id: &str,
        task_id: &str,
        slot: OrderSlot,
        order_key: &str,
    ) -> Result<(), sqlx::Error> {
        pin_dao::set_order_key(&self.pool, user_id, task_id, slot, order_key).await
    }

    async fn clear_snoozed_for_follower(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<(), sqlx::Error> {
        snooze_dao::clear(&self.pool, user_id, task_id).await
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
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}

pub(crate) use super::settle_dao::clear_settled_for_all_followers;

/// Record a deleted Task on the listing feed, and each of its `parents` (read
/// before the delete) so their child counts and Waiting state are re-read.
/// Last in the transaction, per `record_change`'s rule.
async fn record_deletion(
    tx: &mut sqlx::PgConnection,
    task_id: &str,
    parents: &[String],
) -> Result<(), sqlx::Error> {
    moatless_core::change_feed::listing::record_change(tx, task_id).await?;
    for parent in parents {
        moatless_core::change_feed::listing::record_change(tx, parent).await?;
    }
    Ok(())
}

/// The store over a pool. Every fixture in [`crate::task::test_support`] builds
/// on it.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn store_over(pool: &DbPool) -> PgTaskDao {
    PgTaskDao::new(pool.clone())
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_sandbox_row_db_tests.rs"]
mod dao_sandbox_row_db_tests;

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
