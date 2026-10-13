use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::workspace::Workspaces;
use moatless_workspaces::workspace::models::ListedWorkspace;

/// The Workspace reads a T3 answer is built from, over
/// [`moatless_workspaces::workspace::Workspaces`].
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WorkspaceListing: Send + Sync {
    /// Every live Workspace the viewer may read, plus every Workspace named
    /// in `also_include` regardless of access or deletion, ordered by name.
    ///
    /// Widened so a Task naming an unreadable Workspace isn't dropped from a
    /// listing. Each row's [`ListedWorkspace::readable`] says which half it
    /// is — never hand the widened half's config to a refused viewer.
    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<ListedWorkspace>, sqlx::Error>;

    /// The repository a Workspace's branch picker lists, or `None` when the
    /// viewer cannot be shown the Workspace, it is gone, or it declares no
    /// repository.
    async fn primary_repository(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, sqlx::Error>;
}

#[async_trait]
impl WorkspaceListing for Workspaces {
    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<ListedWorkspace>, sqlx::Error> {
        Workspaces::list_readable_including(self, ctx, also_include).await
    }

    async fn primary_repository(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        self.readable_primary_repository(ctx, workspace_id).await
    }
}
