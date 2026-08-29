//! Workspaces' side of the seams declared in `moatless_backend_core::workspace`.
//!
//! `WorkspaceProjection` folds the three stores a Sandbox plan reads — the
//! workspace row, its repository placements, and the plugin activations — into
//! the projection the port publishes, converting `sqlx::Error` into a
//! `StoreError` that boxes it.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use moatless_backend_core::store::StoreError;
use moatless_backend_core::workspace::{
    PluginDelivery, RepoPlacement, ResolvedWorkspace, WorkspaceCredentialReader, WorkspaceResolver,
};
use moatless_types::access_control::ScopeContext;
use uuid::Uuid;

use crate::plugin::activation;
use crate::plugin::dao::PluginDao;
use crate::repository::dao::RepositoryDao;
use crate::workspace::credentials::WorkspaceCredentialService;
use crate::workspace::dao::WorkspaceDao;
use crate::workspace::models::Workspace;

/// The workspace, placement, and plugin stores behind the `WorkspaceResolver`
/// port.
pub struct WorkspaceProjection {
    workspaces: Arc<dyn WorkspaceDao>,
    repositories: Arc<dyn RepositoryDao>,
    plugins: Arc<dyn PluginDao>,
}

impl WorkspaceProjection {
    pub fn new(
        workspaces: Arc<dyn WorkspaceDao>,
        repositories: Arc<dyn RepositoryDao>,
        plugins: Arc<dyn PluginDao>,
    ) -> Self {
        Self {
            workspaces,
            repositories,
            plugins,
        }
    }
}

fn project(workspace: Workspace) -> ResolvedWorkspace {
    ResolvedWorkspace {
        id: workspace.id,
        name: workspace.name,
        docker_image: workspace.docker_image,
        docker_access: workspace.docker_access,
        sandbox_service_account: workspace.sandbox_service_account,
        system_prompt: workspace.system_prompt,
        setup_commands: workspace.setup_commands,
        env_vars: workspace.env_vars,
        servers: workspace.servers,
        resources: workspace.resources,
        network_profile: workspace.network_profile,
        run_as_user_id: workspace.run_as_user_id,
        deleted: workspace.deleted,
    }
}

#[async_trait]
impl WorkspaceResolver for WorkspaceProjection {
    async fn resolve(&self, workspace_id: &str) -> Result<Option<ResolvedWorkspace>, StoreError> {
        // reason: the port resolves across owners — the caller's own
        // authorization already decided the Workspace may be run.
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        let workspace = self
            .workspaces
            .find_by_id(workspace_id, &sys)
            .await
            .map_err(|e| StoreError::with_source(format!("read workspace {workspace_id}"), e))?;
        Ok(workspace.map(project))
    }

    async fn placements(&self, workspace_id: &str) -> Result<Vec<RepoPlacement>, StoreError> {
        let placements = self
            .workspaces
            .find_repos(workspace_id)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read repo placements for workspace {workspace_id}"),
                    e,
                )
            })?;

        // reason: the port resolves across owners, as above.
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        let mut resolved = Vec::with_capacity(placements.len());
        for placement in placements {
            let repository = self
                .repositories
                .find_by_id(&placement.repository_id, &sys)
                .await
                .map_err(|e| {
                    StoreError::with_source(
                        format!("read repository {}", placement.repository_id),
                        e,
                    )
                })?;
            resolved.push(RepoPlacement {
                repository_id: placement.repository_id,
                branch: placement.branch,
                mount_name: placement.mount_name,
                is_primary: placement.is_primary,
                position: placement.position,
                repository,
            });
        }
        Ok(resolved)
    }

    async fn plugin_deliveries(
        &self,
        user_id: Option<Uuid>,
        workspace_id: &str,
    ) -> Result<Vec<PluginDelivery>, StoreError> {
        let resolved = activation::resolve_for(self.plugins.as_ref(), user_id, Some(workspace_id))
            .await
            .map_err(|e| StoreError::with_source("resolve plugin activations", e))?;
        Ok(resolved
            .into_iter()
            .map(|(plugin, skill_names)| PluginDelivery {
                name: plugin.name,
                git_url: plugin.git_url,
                skill_names,
            })
            .collect())
    }
}

#[async_trait]
impl WorkspaceCredentialReader for WorkspaceCredentialService {
    async fn resolve_env(
        &self,
        workspace_id: &str,
    ) -> Result<HashMap<String, String>, StoreError> {
        WorkspaceCredentialService::resolve_env(self, workspace_id)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("resolve pinned credentials for workspace {workspace_id}"),
                    e,
                )
            })
    }
}
