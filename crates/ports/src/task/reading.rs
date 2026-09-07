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
    /// Who started the Task, as a user id. `None` for a Task a Loop fired.
    pub created_by: Option<String>,
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
}
