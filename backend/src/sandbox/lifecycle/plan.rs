//! Lifecycle-owned workspace-to-runtime plan knowledge.
//!
//! Repository seams stay behind the lifecycle module. Runtime adapters receive
//! this bundled dependency and ask it for a fresh provision request; status
//! observation uses the same workspace source for canonical server config.

use std::sync::Arc;

use moatless_backend_core::scope::ScopeContext;

use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::db::DbPool;
use crate::plugin::dao::PluginDao;
use crate::repository::dao::RepositoryDao;
use crate::sandbox::auto_provision::{build_provision_request, get_server_configs_for_task};
use crate::sandbox::models::{ProvisionRequest, ServerConfig};
use crate::task::dao::TaskDao;
use crate::workspace::dao::WorkspaceDao;

use super::error::RuntimeError;
use super::runtime::map_sandbox_error;

/// Repositories needed to assemble fresh runtime plans and status config.
#[derive(Clone)]
pub struct ProvisionDeps {
    tasks: Arc<dyn TaskDao>,
    repos: Arc<dyn RepositoryDao>,
    provider_tokens: Arc<dyn ProviderTokenDao>,
    plugins: Arc<dyn PluginDao>,
    workspaces: Arc<dyn WorkspaceDao>,
}

impl ProvisionDeps {
    pub fn from_pool(pool: &DbPool) -> Self {
        Self {
            tasks: Arc::new(crate::task::dao::PgTaskDao(pool.clone())),
            repos: Arc::new(crate::repository::dao::PgRepositoryDao(pool.clone())),
            provider_tokens: Arc::new(crate::auth::provider_token_dao::PgProviderTokenDao(
                pool.clone(),
            )),
            plugins: Arc::new(crate::plugin::dao::PgPluginDao(
                pool.clone(),
            )),
            workspaces: Arc::new(crate::workspace::dao::PgWorkspaceDao(pool.clone())),
        }
    }

    pub(crate) async fn build_request(
        &self,
        task_id: &str,
    ) -> Result<ProvisionRequest, RuntimeError> {
        build_provision_request(
            self.tasks.as_ref(),
            self.repos.as_ref(),
            self.provider_tokens.as_ref(),
            self.plugins.as_ref(),
            self.workspaces.as_ref(),
            task_id,
        )
        .await
        .map_err(map_sandbox_error)
    }

    pub(crate) async fn server_configs(&self, task_id: &str) -> Vec<ServerConfig> {
        get_server_configs_for_task(self.tasks.as_ref(), self.workspaces.as_ref(), task_id).await
    }

    pub(crate) async fn expected_sandbox_image(
        &self,
        task_id: &str,
        default_image: &str,
    ) -> Result<String, RuntimeError> {
        // reason: image-config lookup resolves across all task owners (lifecycle system path)
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        let task = self
            .tasks
            .find_by_id(task_id, &sys)
            .await
            .map_err(|e| RuntimeError::Infrastructure(format!("read task image config: {e}")))?
            .ok_or_else(|| RuntimeError::Rejected(format!("task {task_id} not found")))?;
        let image = match task.workspace_id.as_deref() {
            Some(workspace_id) => self
                .workspaces
                .find_by_id(workspace_id, &sys)
                .await
                .map_err(|e| {
                    RuntimeError::Infrastructure(format!("read workspace image config: {e}"))
                })?
                .and_then(|workspace| workspace.docker_image)
                .filter(|image| !image.is_empty()),
            None => None,
        };
        Ok(image.unwrap_or_else(|| default_image.to_string()))
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::super::error::ErrorClass;
    use super::super::test_support::seed_task_with;
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test]
    async fn bad_plan_is_permanent_rejected(pool: PgPool) {
        seed_task_with(&pool, "t1", "open", "running").await;
        let error = ProvisionDeps::from_pool(&pool)
            .build_request("t1")
            .await
            .expect_err("missing repository is an unbuildable plan");
        assert!(matches!(error, RuntimeError::Rejected(_)), "got {error:?}");
        assert_eq!(error.class(), ErrorClass::Permanent);
    }
}
