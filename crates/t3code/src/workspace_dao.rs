use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_workspace_types::schemas::workspace::ScriptConfig;

use crate::workspace::UiRpcListedWorkspace;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3WorkspaceDao: Send + Sync {
    /// Every live Workspace the viewer may read, plus every Workspace in
    /// `also_include` whether or not they may read it or it is deleted, ordered
    /// by name.
    ///
    /// The widening covers listings that group Tasks by Workspace: a readable
    /// Task can name an unreadable or tombstoned Workspace, and omitting it
    /// would drop the Task's row too. Each returned row says which half it came
    /// from ([`UiRpcListedWorkspace::readable`]) so a caller cannot hand the
    /// widened half's configuration to someone the read rule refused.
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
