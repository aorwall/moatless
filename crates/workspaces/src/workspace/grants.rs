//! The `WORKSPACE_IDENTITY_GRANTS` entry a Workspace holds, read per request.

use std::sync::Arc;

use moatless_core::workspace_identity_grants::{WorkspaceIdentityGrant, WorkspaceIdentityGrants};
use moatless_types::access_control::{Scope, ScopeContext};

use super::dao::WorkspaceDao;
use crate::repository::dao::RepositoryDao;

/// Matches a Workspace to `WORKSPACE_IDENTITY_GRANTS` the way the git sync
/// does: only a global repository's git-declared Workspace can carry a grant.
pub struct WorkspaceGrants {
    workspaces: Arc<dyn WorkspaceDao>,
    repositories: Arc<dyn RepositoryDao>,
    grants: WorkspaceIdentityGrants,
}

impl WorkspaceGrants {
    pub(crate) fn new(
        workspaces: Arc<dyn WorkspaceDao>,
        repositories: Arc<dyn RepositoryDao>,
        grants: WorkspaceIdentityGrants,
    ) -> Self {
        Self {
            workspaces,
            repositories,
            grants,
        }
    }

    /// True iff `workspace_id`'s grant sets `agentAdmin`.
    pub async fn holds(&self, workspace_id: &str) -> Result<bool, sqlx::Error> {
        if !self.grants.grants_agent_admin() {
            return Ok(false);
        }
        Ok(self
            .grant_for(workspace_id)
            .await?
            .is_some_and(|grant| grant.agent_admin))
    }

    /// The grant `workspace_id` holds, or `None` for a Workspace no grant
    /// names, one that is not git-declared, or one from a non-global
    /// repository.
    pub async fn grant_for(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceIdentityGrant>, sqlx::Error> {
        if !self.grants.grants_proxy_access() {
            return Ok(None);
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "a proxy's scope check runs before any user scope"
        )]
        let sys = ScopeContext::system();
        let Some(workspace) = self.workspaces.find_by_id(workspace_id, &sys).await? else {
            return Ok(None);
        };
        let (Some(repository_id), Some(source_key)) =
            (workspace.source_repository_id, workspace.source_key)
        else {
            return Ok(None);
        };
        let Some(repository) = self.repositories.find_by_id(&repository_id, &sys).await? else {
            return Ok(None);
        };
        let Some(host_repo) = repository
            .remote
            .as_ref()
            .and_then(|remote| remote.host_repo.as_ref())
            .filter(|_| repository.scope == Scope::Global)
        else {
            return Ok(None);
        };
        let full_name = format!("{}/{}", host_repo.owner(), host_repo.name());
        Ok(self.grants.find(&full_name, &source_key).cloned())
    }
}
