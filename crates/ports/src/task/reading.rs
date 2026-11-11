use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;
use moatless_types::task::TaskStatus;

/// The Task facts a peer crate reads. Everything else about a Task — its cost,
/// its run configuration, its sandbox — stays behind the seam.
#[derive(Debug, Clone)]
pub struct TaskDescriptor {
    pub task_id: String,
    pub status: TaskStatus,
    pub name: Option<String>,
    pub owner_user_id: Option<String>,
    /// The Workspace the Task runs in. A peer labelling the Task for a human
    /// resolves this through `WorkspaceResolver`.
    pub workspace_id: Option<String>,
    /// Who the Task is attributed to; a Loop firing uses its run-as user.
    pub created_by: Option<String>,
    /// See [`moatless_types::task::Task::reporter_user_id`].
    pub reporter_user_id: Option<String>,
    /// See [`moatless_types::task::Task::run_as_user_id`].
    pub run_as_user_id: Option<String>,
    /// The Loop that created the Task; `None` for a Task a person started.
    pub loop_id: Option<String>,
    /// How long the Task's Sandbox may sit idle before it is stopped; `0`
    /// disables the idle stop.
    pub ttl_minutes: i32,
}

/// A Task, as the caller may see it.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TaskReader: Send + Sync {
    /// `None` when the Task does not exist or `ctx` may not read it — the same
    /// answer, because a Task a caller cannot read is a Task that is not there.
    async fn describe(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskDescriptor>, StoreError>;

    /// The same predicate as [`Self::describe`], for a caller that needs only
    /// the answer.
    async fn may_access(&self, task_id: &str, ctx: &ScopeContext) -> Result<bool, StoreError>;

    /// The status of the Task [`Self::describe`] would return, for a caller
    /// that polls and needs nothing else off the row.
    async fn readable_status(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskStatus>, StoreError>;

    /// [`Self::describe`], narrowed to a Task `ctx` may write: ownership
    /// widened by public visibility. `None` also for a Task `ctx` can read but
    /// not write, so the answer never says who owns it.
    async fn written(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskDescriptor>, StoreError>;

    /// [`Self::describe`], narrowed to a Task `ctx` owns: owner or admin,
    /// never widened by visibility. `None` as for [`Self::written`].
    async fn owned(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskDescriptor>, StoreError>;
}
