//! Lifecycle-owned workspace-to-runtime plan knowledge.
//!
//! Repository seams stay behind the lifecycle module. Runtime adapters receive
//! this bundled dependency and ask it for a fresh provision request; status
//! observation uses the same workspace source for canonical server config.

use std::sync::Arc;

use moatless_types::access_control::ScopeContext;

use crate::auto_provision::{build_provision_request, get_server_configs_for_task};
use crate::models::ProvisionRequest;
use moatless_types::workspace::ServerConfig;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::auth::users::UserDao;
use moatless_tasks::task::dao::TaskDao;
use moatless_workspaces::plugin::dao::PluginDao;
use moatless_workspaces::repository::dao::RepositoryDao;
use moatless_workspaces::workspace::dao::WorkspaceDao;

use super::error::RuntimeError;
use super::runtime::map_sandbox_error;

/// Repositories needed to assemble fresh runtime plans and status config.
/// `users` resolves the platform-admin flag for the control check every plan is
/// gated on.
#[derive(Clone)]
pub struct ProvisionDeps {
    tasks: Arc<dyn TaskDao>,
    repos: Arc<dyn RepositoryDao>,
    provider_tokens: Arc<dyn ProviderTokenDao>,
    plugins: Arc<dyn PluginDao>,
    workspaces: Arc<dyn WorkspaceDao>,
    users: Arc<dyn UserDao>,
}

impl ProvisionDeps {
    /// Assemble from the composed ports. The composition root is the only place
    /// a `Pg*Dao` is constructed, so this takes the six ports it needs rather
    /// than a pool.
    pub fn new(
        tasks: Arc<dyn TaskDao>,
        repos: Arc<dyn RepositoryDao>,
        provider_tokens: Arc<dyn ProviderTokenDao>,
        plugins: Arc<dyn PluginDao>,
        workspaces: Arc<dyn WorkspaceDao>,
        users: Arc<dyn UserDao>,
    ) -> Self {
        Self {
            tasks,
            repos,
            provider_tokens,
            plugins,
            workspaces,
            users,
        }
    }

    pub async fn build_request(&self, sandbox_id: &str) -> Result<ProvisionRequest, RuntimeError> {
        build_provision_request(
            self.tasks.as_ref(),
            self.repos.as_ref(),
            self.provider_tokens.as_ref(),
            self.plugins.as_ref(),
            self.workspaces.as_ref(),
            self.users.as_ref(),
            sandbox_id,
        )
        .await
        .map_err(map_sandbox_error)
    }

    pub async fn sandbox_id_for_task(&self, task_id: &str) -> Result<String, RuntimeError> {
        // reason: endpoint resolution is a system read of task membership.
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        let task = self
            .tasks
            .find_by_id(task_id, &sys)
            .await
            .map_err(|e| RuntimeError::Infrastructure(format!("read task sandbox id: {e}")))?
            .ok_or_else(|| RuntimeError::Rejected(format!("task {task_id} not found")))?;
        Ok(task.sandbox_id.unwrap_or_else(|| task.task_id.clone()))
    }

    pub async fn server_configs(&self, task_id: &str) -> Vec<ServerConfig> {
        get_server_configs_for_task(self.tasks.as_ref(), self.workspaces.as_ref(), task_id).await
    }

    pub async fn expected_sandbox_image(
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

    /// Assemble deps straight from a test pool, bypassing the composition root
    /// the same way the `db-tests` suite always has.
    fn deps_from_pool(pool: &PgPool) -> ProvisionDeps {
        ProvisionDeps::new(
            Arc::new(moatless_tasks::task::dao::PgTaskDao(pool.clone())),
            Arc::new(moatless_workspaces::repository::dao::PgRepositoryDao(
                pool.clone(),
            )),
            Arc::new(moatless_identity::auth::provider_token_dao::PgProviderTokenDao(pool.clone())),
            Arc::new(moatless_workspaces::plugin::dao::PgPluginDao(pool.clone())),
            Arc::new(moatless_workspaces::workspace::dao::PgWorkspaceDao(
                pool.clone(),
            )),
            Arc::new(moatless_identity::auth::users::PgUserDao(pool.clone())),
        )
    }

    #[sqlx::test]
    async fn bad_plan_is_permanent_rejected(pool: PgPool) {
        seed_task_with(&pool, "t1", "open", "running").await;
        let error = deps_from_pool(&pool)
            .build_request("t1")
            .await
            .expect_err("missing repository is an unbuildable plan");
        assert!(matches!(error, RuntimeError::Rejected(_)), "got {error:?}");
        assert_eq!(error.class(), ErrorClass::Permanent);
    }
}
