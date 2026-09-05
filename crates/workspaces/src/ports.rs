//! Workspaces' side of the seams declared in `moatless_ports::workspace`.
//!
//! `WorkspaceProjection` folds the three stores a Sandbox plan reads — the
//! workspace row, its repository placements, and the plugin activations — into
//! the projection the port publishes, converting `sqlx::Error` into a
//! `StoreError` that boxes it.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use moatless_ports::workspace::{
    PluginDelivery, RepoPlacement, RepositoryReader, ResolvedWorkspace, WorkspaceCredentialReader,
    WorkspaceProvisioner, WorkspaceResolver,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::repository::Repository;
use moatless_types::store::StoreError;
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
            .map(|(plugin, skills)| PluginDelivery {
                name: plugin.name,
                git_url: plugin.git_url,
                skill_names: skills.names,
                frontload_skill_names: skills.frontloaded,
            })
            .collect())
    }

    async fn names(&self, workspace_ids: &[String]) -> Result<HashMap<String, String>, StoreError> {
        if workspace_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let pairs = self
            .workspaces
            .find_names_by_ids(workspace_ids)
            .await
            .map_err(|e| StoreError::with_source("batch read workspace names", e))?;
        Ok(pairs.into_iter().collect())
    }

    async fn describe(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<Workspace>, StoreError> {
        self.workspaces
            .find_by_id(workspace_id, ctx)
            .await
            .map_err(|e| StoreError::with_source(format!("read workspace {workspace_id}"), e))
    }
}

/// The `repositories` store, narrowed to what a peer band reads.
pub struct RepositoryProjection {
    repositories: Arc<dyn RepositoryDao>,
}

impl RepositoryProjection {
    pub fn new(repositories: Arc<dyn RepositoryDao>) -> Self {
        Self { repositories }
    }
}

#[async_trait]
impl RepositoryReader for RepositoryProjection {
    async fn describe(
        &self,
        ctx: &ScopeContext,
        repository_id: &str,
    ) -> Result<Option<Repository>, StoreError> {
        self.repositories
            .find_by_id(repository_id, ctx)
            .await
            .map_err(|e| StoreError::with_source(format!("read repository {repository_id}"), e))
    }

    async fn names(
        &self,
        repository_ids: &[String],
    ) -> Result<HashMap<String, String>, StoreError> {
        if repository_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let pairs = self
            .repositories
            .find_names_by_ids(repository_ids)
            .await
            .map_err(|e| StoreError::with_source("batch read repository names", e))?;
        Ok(pairs.into_iter().collect())
    }
}

/// The shadow Workspace a repository-only Task is persisted against.
///
/// Holds the pool rather than a `*Dao` because the write is one statement pair
/// in a transaction, which no row-at-a-time store method expresses.
pub struct ShadowWorkspaces(moatless_core::db::DbPool);

impl ShadowWorkspaces {
    pub fn new(pool: moatless_core::db::DbPool) -> Self {
        Self(pool)
    }
}

#[async_trait]
impl WorkspaceProvisioner for ShadowWorkspaces {
    async fn ensure_shadow(&self, repository_id: &str) -> Result<String, StoreError> {
        crate::workspace::dao::ensure_shadow_workspace(&self.0, repository_id)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("ensure the shadow workspace for repository {repository_id}"),
                    e,
                )
            })
    }
}

#[async_trait]
impl WorkspaceCredentialReader for WorkspaceCredentialService {
    async fn resolve_env(&self, workspace_id: &str) -> Result<HashMap<String, String>, StoreError> {
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
