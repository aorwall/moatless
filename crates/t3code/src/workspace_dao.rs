use async_trait::async_trait;
use moatless_api_schemas::workspace::ScriptConfig;
use moatless_types::access_control::ScopeContext;

use crate::workspace::UiRpcListedWorkspace;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3WorkspaceDao: Send + Sync {
    /// Every live Workspace the viewer may read, plus every Workspace in
    /// `also_include` whether or not they may read it and whether or not it is
    /// deleted, ordered by name.
    ///
    /// The widening exists for the listings that group Tasks by Workspace: a
    /// Task the viewer may read can name someone else's private Workspace, or
    /// one a git sync has tombstoned, and a grouping that omits the Workspace
    /// omits the Task with it — the sidebar loses the row and the link is
    /// refused, which is what `snapshots::readable_under_a_project` decides.
    ///
    /// Each row says which half it came from — see
    /// [`UiRpcListedWorkspace::readable`]. A caller that projects the two halves
    /// the same way hands the widened half's configuration to someone the read
    /// rule refused, so the distinction is returned rather than re-derived.
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
