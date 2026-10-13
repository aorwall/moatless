use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_workspace_types::schemas::workspace::ScriptConfig;

use crate::workspace::UiRpcListedWorkspace;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3WorkspaceDao: Send + Sync {
    /// Every live Workspace the viewer may read, plus every Workspace named
    /// in `also_include` regardless of access or deletion, ordered by name.
    ///
    /// Widened so a Task naming an unreadable Workspace isn't dropped from a
    /// listing. Each row's [`UiRpcListedWorkspace::readable`] says which
    /// half it is — never hand the widened half's config to a refused viewer.
    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<UiRpcListedWorkspace>, sqlx::Error>;

    /// The repository a Workspace's branch picker lists, or `None` when the
    /// viewer cannot be shown the Workspace, it is gone, or it declares no
    /// repository.
    ///
    /// One answer rather than an existence check and a read: all three cases
    /// are one answer to the caller — there is no list to show.
    async fn primary_repository(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, sqlx::Error>;

    /// The Scripts declared by the Workspace a Task runs against, scoped to the
    /// viewer.
    ///
    /// `None` when the Task is not readable or does not exist — the same answer
    /// as everywhere else a scoped read finds nothing; `Some(vec)` (maybe
    /// empty) when it is readable.
    async fn scripts_for_task(
        &self,
        ctx: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Vec<ScriptConfig>>, sqlx::Error>;
}
