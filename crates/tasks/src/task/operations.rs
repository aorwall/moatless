//! The `tasks` table, as a host and the task routes reach it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::dao::TaskDao;
use super::models::{SandboxIntentView, Task, TaskFilter, TaskPatch, TaskStatus};
use moatless_types::access_control::ScopeContext;

/// The `tasks`, `task_follows` and `task_read_receipts` tables, as the backend
/// and handlers reach them. Narrower published seams:
/// [`crate::ports::TaskProjection`] for reads, `TaskAccess` for the gate.
pub struct TaskCommands {
    store: Arc<dyn TaskDao>,
}

impl TaskCommands {
    pub(crate) fn new(store: Arc<dyn TaskDao>) -> Self {
        Self { store }
    }

    /// See [`TaskDao::find_by_id`].
    pub async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Task>, sqlx::Error> {
        self.store.find_by_id(task_id, ctx).await
    }

    /// See [`TaskDao::insert_task`].
    pub async fn insert_task(
        &self,
        row: &Task,
        placement: &super::models::SandboxPlacement,
    ) -> Result<(), sqlx::Error> {
        self.store.insert_task(row, placement).await
    }

    /// See [`TaskDao::delete_inserted_task_after_join_failure`].
    pub async fn delete_inserted_task_after_join_failure(
        &self,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .delete_inserted_task_after_join_failure(task_id)
            .await
    }

    /// See [`TaskDao::delete_errored_empty_task`].
    pub async fn delete_errored_empty_task(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store.delete_errored_empty_task(task_id, ctx).await
    }

    /// See [`TaskDao::list_tasks`].
    pub async fn list_tasks(&self, filter: &TaskFilter) -> Result<(Vec<Task>, i64), sqlx::Error> {
        self.store.list_tasks(filter).await
    }

    /// See [`TaskDao::update_task`].
    pub async fn update_task(
        &self,
        task_id: &str,
        patch: &TaskPatch,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store.update_task(task_id, patch, ctx).await
    }

    /// See [`TaskDao::recompute_total_cost`].
    pub async fn recompute_total_cost(&self, task_id: &str) -> Result<(), sqlx::Error> {
        self.store.recompute_total_cost(task_id).await
    }

    /// See [`TaskDao::update_status_if_not_closed`].
    pub async fn update_status_if_not_closed(
        &self,
        task_id: &str,
        new_status: TaskStatus,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .update_status_if_not_closed(task_id, new_status, ctx)
            .await
    }

    /// See [`TaskDao::delete_task`].
    pub async fn delete_task(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store.delete_task(task_id, ctx).await
    }

    /// See [`TaskDao::exists_by_short_id`].
    pub async fn exists_by_short_id(&self, short_id: &str) -> Result<bool, sqlx::Error> {
        self.store.exists_by_short_id(short_id).await
    }

    /// See [`TaskDao::get_created_times`].
    pub async fn get_created_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        self.store.get_created_times(task_ids).await
    }

    /// See [`TaskDao::get_sandbox_started_times`].
    pub async fn get_sandbox_started_times(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        self.store.get_sandbox_started_times(task_ids).await
    }

    /// See [`TaskDao::get_sandbox_intents`].
    pub async fn get_sandbox_intents(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, SandboxIntentView>, sqlx::Error> {
        self.store.get_sandbox_intents(task_ids).await
    }

    /// See [`TaskDao::get_ttl_minutes_map`].
    pub async fn get_ttl_minutes_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        self.store.get_ttl_minutes_map(task_ids).await
    }

    /// See [`TaskDao::get_inactive_ttl_days_map`].
    pub async fn get_inactive_ttl_days_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        self.store.get_inactive_ttl_days_map(task_ids).await
    }

    /// See [`TaskDao::get_cold_after_hours_map`].
    pub async fn get_cold_after_hours_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        self.store.get_cold_after_hours_map(task_ids).await
    }

    /// See [`TaskDao::list_open_task_ids`].
    pub async fn list_open_task_ids(&self) -> Result<Vec<String>, sqlx::Error> {
        self.store.list_open_task_ids().await
    }

    /// See [`TaskDao::list_sandbox_members`].
    pub async fn list_sandbox_members(
        &self,
        sandbox_ids: &[String],
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        self.store.list_sandbox_members(sandbox_ids).await
    }

    /// See [`TaskDao::sandbox_to_stop_when_done`].
    pub async fn sandbox_to_stop_when_done(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        self.store.sandbox_to_stop_when_done(task_id).await
    }

    /// See [`TaskDao::mark_task_read`].
    pub async fn mark_task_read(
        &self,
        user_id: &str,
        task_id: &str,
        read_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        self.store.mark_task_read(user_id, task_id, read_at).await
    }

    /// See [`TaskDao::get_read_receipts`].
    pub async fn get_read_receipts(
        &self,
        user_id: &str,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        self.store.get_read_receipts(user_id, task_ids).await
    }

    /// See [`TaskDao::follow_task`].
    pub async fn follow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        self.store.follow_task(user_id, task_id).await
    }

    /// See [`TaskDao::unfollow_task`].
    pub async fn unfollow_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        self.store.unfollow_task(user_id, task_id).await
    }

    /// See [`TaskDao::settle_task`].
    pub async fn settle_task(
        &self,
        user_id: &str,
        task_id: &str,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        self.store.settle_task(user_id, task_id, at).await
    }

    /// See [`TaskDao::unsettle_task`].
    pub async fn unsettle_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        self.store.unsettle_task(user_id, task_id).await
    }

    /// See [`TaskDao::snooze_task`].
    pub async fn snooze_task(
        &self,
        user_id: &str,
        task_id: &str,
        until: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        self.store.snooze_task(user_id, task_id, until, at).await
    }

    /// See [`TaskDao::unsnooze_task`].
    pub async fn unsnooze_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        self.store.unsnooze_task(user_id, task_id).await
    }

    /// See [`TaskDao::clear_snoozed_for_follower`].
    pub async fn clear_snoozed_for_follower(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<(), sqlx::Error> {
        self.store
            .clear_snoozed_for_follower(user_id, task_id)
            .await
    }

    /// See [`TaskDao::get_followed`].
    pub async fn get_followed(
        &self,
        user_id: &str,
        task_ids: &[String],
    ) -> Result<HashSet<String>, sqlx::Error> {
        self.store.get_followed(user_id, task_ids).await
    }

    /// See [`TaskDao::list_open_codex_tasks_for_scope`].
    pub async fn list_open_codex_tasks_for_scope(
        &self,
        scope_kind: String,
        scope_id: String,
        except_task_id: String,
    ) -> Result<Vec<String>, sqlx::Error> {
        self.store
            .list_open_codex_tasks_for_scope(scope_kind, scope_id, except_task_id)
            .await
    }
}
