//! Whether a Workspace holds an `agentAdmin` grant.

use std::sync::Arc;

use moatless_core::workspace_identity_grants::WorkspaceIdentityGrants;
use moatless_types::access_control::{Scope, ScopeContext};

use super::dao::WorkspaceDao;
use crate::repository::dao::RepositoryDao;

/// Matches a Workspace to `WORKSPACE_IDENTITY_GRANTS` the way the git sync
/// does: only a global repository's git-declared Workspace can carry a grant.
pub struct AgentAdminGrants {
    workspaces: Arc<dyn WorkspaceDao>,
    repositories: Arc<dyn RepositoryDao>,
    grants: WorkspaceIdentityGrants,
}

impl AgentAdminGrants {
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
        // reason: the sandbox token's own scope check asks, before any user
        // scope applies.
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        let Some(workspace) = self.workspaces.find_by_id(workspace_id, &sys).await? else {
            return Ok(false);
        };
        let (Some(repository_id), Some(source_key)) =
            (workspace.source_repository_id, workspace.source_key)
        else {
            return Ok(false);
        };
        let Some(repository) = self.repositories.find_by_id(&repository_id, &sys).await? else {
            return Ok(false);
        };
        let Some(host_repo) = repository
            .remote
            .as_ref()
            .and_then(|remote| remote.host_repo.as_ref())
            .filter(|_| repository.scope == Scope::Global)
        else {
            return Ok(false);
        };
        let full_name = format!("{}/{}", host_repo.owner(), host_repo.name());
        Ok(self
            .grants
            .find(&full_name, &source_key)
            .is_some_and(|grant| grant.agent_admin))
    }
}
