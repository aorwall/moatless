//! Lifecycle-owned workspace-to-runtime plan knowledge.
//!
//! Repository seams stay behind the lifecycle module. Runtime adapters receive
//! this bundled dependency and ask it for a fresh provision request.

use std::sync::Arc;

use crate::auto_provision::build_provision_request;
use moatless_core::sandbox_identities::RoleIdentities;
use moatless_ports::identity::CommitIdentity;
use moatless_sandbox_runtime::provisioning::ProvisionRequest;
use moatless_workspace_ports::WorkspaceResolver;

use super::dao::LifecycleDao;
use super::orchestrator::TaskIntent;
use moatless_sandbox_runtime::runtime::RuntimeError;
use moatless_sandbox_runtime::runtime::map_sandbox_error;

/// The three seams a fresh runtime plan is assembled from.
///
/// `sandboxes` is the sole source of a pod's shape, `workspaces` supplies run
/// configuration and repository placements, and `users` resolves the
/// platform-admin flag every plan is gated on, and `roles` picks the identity
/// a sandbox whose Workspace names none runs as.
#[derive(Clone)]
pub struct ProvisionDeps {
    sandboxes: Arc<dyn LifecycleDao>,
    workspaces: Arc<dyn WorkspaceResolver>,
    users: Arc<dyn CommitIdentity>,
    roles: Arc<RoleIdentities>,
}

impl ProvisionDeps {
    pub(crate) fn new(
        sandboxes: Arc<dyn LifecycleDao>,
        workspaces: Arc<dyn WorkspaceResolver>,
        users: Arc<dyn CommitIdentity>,
        roles: RoleIdentities,
    ) -> Self {
        Self {
            sandboxes,
            workspaces,
            users,
            roles: Arc::new(roles),
        }
    }

    pub async fn build_request(&self, sandbox_id: &str) -> Result<ProvisionRequest, RuntimeError> {
        build_provision_request(
            self.sandboxes.as_ref(),
            self.workspaces.as_ref(),
            self.users.as_ref(),
            &self.roles,
            sandbox_id,
        )
        .await
        .map_err(map_sandbox_error)
    }

    pub async fn sandbox_id_for_task(&self, task_id: &str) -> Result<String, RuntimeError> {
        Ok(self.intent(task_id).await?.sandbox_id)
    }

    pub async fn expected_sandbox_image(
        &self,
        task_id: &str,
        default_image: &str,
    ) -> Result<String, RuntimeError> {
        let workspace_id = self.intent(task_id).await?.workspace_id;
        let image = self
            .workspaces
            .resolve(&workspace_id)
            .await
            .map_err(|e| RuntimeError::Infrastructure(format!("read workspace image config: {e}")))?
            .and_then(|workspace| workspace.docker_image)
            .filter(|image| !image.is_empty());
        Ok(image.unwrap_or_else(|| default_image.to_string()))
    }

    /// The Task's Sandbox membership. A Task with no row is `Rejected`: the
    /// caller named something that is not a member of anything.
    async fn intent(&self, task_id: &str) -> Result<TaskIntent, RuntimeError> {
        self.sandboxes
            .load(task_id)
            .await
            .map_err(|e| RuntimeError::Infrastructure(format!("read sandbox membership: {e}")))?
            .ok_or_else(|| RuntimeError::Rejected(format!("task {task_id} not found")))
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::super::test_support::seed_task_with;
    use super::*;
    use moatless_sandbox_runtime::runtime::ErrorClass;
    use sqlx::PgPool;

    /// Assemble deps over a test pool through the crate's own composition root.
    fn deps_from_pool(pool: &PgPool) -> ProvisionDeps {
        crate::compose::sandbox_graph(pool.clone()).provision_deps(
            moatless_workspaces::compose::workspace_resolver(pool.clone()),
            // The seeded task is user-scoped and owned by its creator, so the
            // control gate never reaches for the platform-admin flag.
            Arc::new(non_admin()),
            RoleIdentities::default(),
        )
    }

    /// A `CommitIdentity` that grants nobody platform admin.
    fn non_admin() -> moatless_ports::identity::MockCommitIdentity {
        let mut users = moatless_ports::identity::MockCommitIdentity::new();
        users.expect_is_platform_admin().returning(|_| Ok(false));
        users
    }

    #[sqlx::test]
    async fn bad_plan_is_permanent_rejected(pool: PgPool) {
        seed_task_with(&pool, "t1", "open", "running").await;
        sqlx::query(
            "INSERT INTO workspace_repos (id, workspace_id, repository_id, is_primary)
             VALUES ('wr-gone', 'ws_repo-x', 'repo-gone', true)",
        )
        .execute(&pool)
        .await
        .expect("seed a placement whose repository is gone");
        let error = deps_from_pool(&pool)
            .build_request("t1")
            .await
            .expect_err("a placement naming a missing repository is an unbuildable plan");
        assert!(matches!(error, RuntimeError::Rejected(_)), "got {error:?}");
        assert_eq!(error.class(), ErrorClass::Permanent);
    }

    #[sqlx::test]
    async fn a_workspace_placing_no_repository_plans_an_empty_checkout(pool: PgPool) {
        seed_task_with(&pool, "t1", "open", "running").await;
        let request = deps_from_pool(&pool)
            .build_request("t1")
            .await
            .expect("a workspace with no placements still provisions");
        assert!(request.plan.repos.is_empty());
        assert_eq!(request.plan.git_clone_url, "");
    }
}
