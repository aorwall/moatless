// `automock` names one field per trait method, and all three of these read a
// Task by id.
#![cfg_attr(test, allow(clippy::struct_field_names))]

use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::TaskStatus;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskView {
    #[sqlx(try_from = "String")]
    pub status: TaskStatus,
    /// How long the Task's Sandbox may sit idle before the reaper stops it. `0`
    /// disables the idle stop; `moatless_tasks::task::models` owns the rule.
    pub ttl_minutes: i32,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3TaskDao: Send + Sync {
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error>;

    /// The same read, narrowed to Tasks this viewer may **operate** (the write
    /// rule widened by `visibility = 'public'`).
    ///
    /// `None` collapses gone, unreadable, and not-runnable into one answer —
    /// distinguishing them would say who owns it.
    async fn controlled_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error>;

    /// The same read, narrowed to Tasks this viewer may **write**: the grant a
    /// change the owner would have to undo needs, which `visibility = 'public'`
    /// does not widen. `None` for the same three cases, for the same reason.
    async fn written_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error>;
}
