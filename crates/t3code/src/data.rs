use async_trait::async_trait;
use moatless_api_schemas::enums::TaskStatus;
use moatless_api_schemas::repository::ScriptConfig;
use moatless_backend_core::scope::ScopeContext;

use crate::workspace::UiRpcListedWorkspace;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskView {
    pub status: TaskStatus,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3WorkspaceDao: Send + Sync {
    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<UiRpcListedWorkspace>, sqlx::Error>;

    /// The repository a Workspace's branch picker lists, or `None` when the
    /// viewer cannot be shown the Workspace, it is gone, or it declares no
    /// repository. One answer rather than an existence check and a read: the
    /// three cases are one answer to the caller — there is no list to show.
    async fn primary_repository(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, sqlx::Error>;

    /// The Scripts declared by the Workspace a Task runs against, scoped to the
    /// viewer. `None` when the Task is not readable or does not exist — the same
    /// answer as everywhere else a scoped read finds nothing; `Some(vec)` (which
    /// may be empty) when it is readable.
    async fn scripts_for_task(
        &self,
        ctx: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Vec<ScriptConfig>>, sqlx::Error>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3TaskDao: Send + Sync {
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error>;
}
