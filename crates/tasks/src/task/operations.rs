//! The `tasks` table, as a host and the task routes reach it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::dao::TaskDao;
use super::models::{SandboxIntentView, Task, TaskFilter, TaskPatch, TaskStatus};
use super::pin_dao::OrderSlot;
use moatless_types::access_control::{ScopeContext, ScopedResource};

/// Who holds a caller-minted Task id, as [`TaskCommands::claim_minted_id`]
/// answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MintedIdClaim {
    /// No Task this viewer can read holds it. One they cannot read may, and the
    /// create's unique index refuses it without confirming the id is taken.
    Free,
    /// This viewer's empty `Error` Task held it, and was deleted.
    Reclaimed,
    /// A Task this viewer may read but not write holds it.
    NotWritable,
    /// A Task this viewer may write holds it, and it is not an empty `Error`
    /// one.
    Taken,
}

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

    /// See [`TaskDao::delete_errored_empty_task`].
    pub async fn delete_errored_empty_task(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store.delete_errored_empty_task(task_id, ctx).await
    }

    /// Make a caller-minted Task id available to this viewer's create.
    ///
    /// A client that mints its own ids retries a failed creation under the same
    /// id, which collides with the `Error` row the failure left; nothing else
    /// can delete that row, so reclaiming it here is the only unblock.
    pub async fn claim_minted_id(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<MintedIdClaim, sqlx::Error> {
        let Some(existing) = self.store.find_by_id(task_id, viewer).await? else {
            return Ok(MintedIdClaim::Free);
        };
        if !viewer.is_owner(&ScopedResource::task(
            existing
                .owner_user_id
                .as_deref()
                .or(existing.created_by.as_deref()),
        )) {
            return Ok(MintedIdClaim::NotWritable);
        }
        if self
            .store
            .delete_errored_empty_task(task_id, viewer)
            .await?
        {
            Ok(MintedIdClaim::Reclaimed)
        } else {
            Ok(MintedIdClaim::Taken)
        }
    }

    /// Whether `viewer` may read `task_id`, it is open, and they follow it: the
    /// gate on changing their own settle, snooze and pin state for it.
    pub async fn followed_and_open(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let Some(task) = self.store.find_by_id(task_id, viewer).await? else {
            return Ok(false);
        };
        if task.status != TaskStatus::Open {
            return Ok(false);
        }
        let followed = self
            .store
            .get_followed(&viewer.user_id, &[task_id.to_string()])
            .await?;
        Ok(followed.contains(task_id))
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

    /// Claim an unused Task for deletion without counting inherited conversation.
    pub async fn close_if_unused(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store.close_if_unused(task_id, ctx).await
    }

    pub(crate) async fn side_chat_ids(&self, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
        self.store.side_chat_ids(task_id).await
    }

    /// See [`TaskDao::delete_task`].
    pub async fn delete_task(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store.delete_task(task_id, ctx).await
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

    /// See [`TaskDao::get_close_after_settled_days_map`].
    pub async fn get_close_after_settled_days_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        self.store.get_close_after_settled_days_map(task_ids).await
    }

    /// See [`TaskDao::get_settle_after_hours_map`].
    pub async fn get_settle_after_hours_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, i32>, sqlx::Error> {
        self.store.get_settle_after_hours_map(task_ids).await
    }

    /// See [`TaskDao::get_settled_since_map`].
    pub async fn get_settled_since_map(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        self.store.get_settled_since_map(task_ids).await
    }

    /// See [`TaskDao::auto_settle_task`].
    pub async fn auto_settle_task(
        &self,
        task_id: &str,
        at: DateTime<Utc>,
        idle_since: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        self.store.auto_settle_task(task_id, at, idle_since).await
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

    /// See [`TaskDao::mark_task_unread`].
    pub async fn mark_task_unread(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        self.store.mark_task_unread(user_id, task_id).await
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

    /// See [`TaskDao::set_auto_settle`].
    pub async fn set_auto_settle(
        &self,
        user_id: &str,
        task_id: &str,
        enabled: bool,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        self.store
            .set_auto_settle(user_id, task_id, enabled, at)
            .await
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
    pub async fn unsnooze_task(
        &self,
        user_id: &str,
        task_id: &str,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        self.store.unsnooze_task(user_id, task_id, at).await
    }

    /// See [`TaskDao::pin_task`].
    pub async fn pin_task(
        &self,
        user_id: &str,
        task_id: &str,
        order_key: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        self.store
            .pin_task(user_id, task_id, order_key.map(str::to_string), at)
            .await
    }

    /// See [`TaskDao::unpin_task`].
    pub async fn unpin_task(&self, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        self.store.unpin_task(user_id, task_id).await
    }

    /// See [`TaskDao::set_order_key`].
    pub async fn set_pin_order_key(
        &self,
        user_id: &str,
        task_id: &str,
        order_key: &str,
    ) -> Result<(), sqlx::Error> {
        self.store
            .set_order_key(user_id, task_id, OrderSlot::Pinned, order_key)
            .await
    }

    /// See [`TaskDao::set_order_key`].
    pub async fn set_active_order_key(
        &self,
        user_id: &str,
        task_id: &str,
        order_key: &str,
    ) -> Result<(), sqlx::Error> {
        self.store
            .set_order_key(user_id, task_id, OrderSlot::Active, order_key)
            .await
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::dao::MockTaskDao;
    use crate::task::test_support::make_task_row;

    fn commands(task: Option<Task>, followed: bool) -> TaskCommands {
        let mut store = MockTaskDao::new();
        store
            .expect_find_by_id()
            .returning(move |_, _| Ok(task.clone()));
        store.expect_get_followed().returning(move |_, ids| {
            Ok(if followed {
                ids.iter().cloned().collect()
            } else {
                HashSet::new()
            })
        });
        TaskCommands::new(Arc::new(store))
    }

    #[tokio::test]
    async fn only_an_open_readable_followed_task_takes_follower_state() {
        let viewer = ScopeContext::user("u-1");
        let open = make_task_row("t-1", "u-1");
        let mut closed = open.clone();
        closed.status = TaskStatus::Closed;

        for (task, followed, expected) in [
            (Some(open.clone()), true, true),
            (Some(open), false, false),
            (Some(closed), true, false),
            (None, true, false),
        ] {
            let allowed = commands(task, followed)
                .followed_and_open(&viewer, "t-1")
                .await
                .unwrap();
            assert_eq!(allowed, expected);
        }
    }

    #[tokio::test]
    async fn a_minted_id_is_reclaimed_only_from_the_viewers_own_errored_empty_task() {
        let viewer = ScopeContext::user("u-1");
        for (owner, row, deleted, expected) in [
            ("u-1", false, false, MintedIdClaim::Free),
            ("u-2", true, false, MintedIdClaim::NotWritable),
            ("u-1", true, true, MintedIdClaim::Reclaimed),
            ("u-1", true, false, MintedIdClaim::Taken),
        ] {
            let task = row.then(|| make_task_row("t-1", owner));
            let mut store = MockTaskDao::new();
            store
                .expect_find_by_id()
                .returning(move |_, _| Ok(task.clone()));
            store
                .expect_delete_errored_empty_task()
                .returning(move |_, _| Ok(deleted));
            let claim = TaskCommands::new(Arc::new(store))
                .claim_minted_id(&viewer, "t-1")
                .await
                .unwrap();
            assert_eq!(
                claim, expected,
                "owner {owner}, row {row}, deleted {deleted}"
            );
        }
    }
}
