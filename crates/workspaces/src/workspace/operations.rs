//! The workspace capability a host holds.

use std::sync::Arc;

use moatless_types::access_control::ScopeContext;

use super::dao::WorkspaceDao;
use super::models::{ListedWorkspace, WorkspaceRepo};
#[cfg(any(test, feature = "test-support"))]
use super::models::{UpdateOutcome, Workspace, WorkspacePatch};

/// The `workspaces` and `workspace_repos` tables, as a host and this crate's
/// own routes reach them.
///
/// The store stays private to this crate; a peer band that resolves a workspace
/// to run a Sandbox takes [`moatless_workspace_ports::WorkspaceResolver`]
/// instead.
pub struct Workspaces {
    store: Arc<dyn WorkspaceDao>,
}

impl Workspaces {
    pub(crate) fn new(store: Arc<dyn WorkspaceDao>) -> Self {
        Self { store }
    }

    /// The store, for this crate's own handlers and mutations.
    pub(crate) fn store(&self) -> &dyn WorkspaceDao {
        self.store.as_ref()
    }

    /// The workspace's primary repo placement, falling back to the
    /// lowest-`position` placement when none is flagged primary.
    pub async fn find_primary_repo(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRepo>, sqlx::Error> {
        self.store.find_primary_repo(workspace_id).await
    }

    /// Every workspace the caller may read, widened by the ids in
    /// `also_include`. See [`WorkspaceDao::list_readable_including`] for what
    /// each row states about the half it came from.
    pub async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<ListedWorkspace>, sqlx::Error> {
        self.store.list_readable_including(ctx, also_include).await
    }
}

/// The surface a consumer's PostgreSQL tests seed and read a workspace through.
#[cfg(any(test, feature = "test-support"))]
impl Workspaces {
    pub async fn insert_workspace(&self, row: &Workspace) -> Result<(), sqlx::Error> {
        self.store.insert_workspace(row).await
    }

    pub async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Workspace>, sqlx::Error> {
        self.store.find_by_id(id, ctx).await
    }

    pub async fn add_repo(
        &self,
        row: &WorkspaceRepo,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store.add_repo(row, ctx).await
    }

    pub async fn update(
        &self,
        id: &str,
        patch: &WorkspacePatch,
        ctx: &ScopeContext,
    ) -> Result<UpdateOutcome, sqlx::Error> {
        self.store.update(id, patch, ctx).await
    }

    pub async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        self.store.soft_delete(id, ctx).await
    }

    pub async fn soft_delete_git_origin(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        self.store.soft_delete_git_origin(id, ctx).await
    }
}
