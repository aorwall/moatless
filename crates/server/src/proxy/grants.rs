//! The `WORKSPACE_IDENTITY_GRANTS` entry a proxy reads per request.

use async_trait::async_trait;
use moatless_core::workspace_identity_grants::WorkspaceIdentityGrant;

/// Reads the `WORKSPACE_IDENTITY_GRANTS` entry a Workspace holds.
#[async_trait]
pub trait GrantReader: Send + Sync {
    async fn grant_for(&self, workspace_id: &str)
    -> Result<Option<WorkspaceIdentityGrant>, String>;
}

#[async_trait]
impl GrantReader for moatless_workspaces::workspace::grants::WorkspaceGrants {
    async fn grant_for(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceIdentityGrant>, String> {
        Self::grant_for(self, workspace_id)
            .await
            .map_err(|e| e.to_string())
    }
}
