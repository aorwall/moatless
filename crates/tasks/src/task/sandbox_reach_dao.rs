//! The reads behind [`super::sandbox_reach::SandboxReachStore`].

use moatless_types::task::TaskRelationType;

use super::dao::PgTaskDao;

impl PgTaskDao {
    /// True iff `task_id` runs in `sandbox_id`, or a task running there
    /// recorded a `created_by` edge to it.
    pub(super) async fn in_sandbox_reach(
        &self,
        task_id: &str,
        sandbox_id: &str,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM tasks t
                 WHERE t.task_id = $1
                   AND (t.sandbox_id = $2 OR EXISTS (
                       SELECT 1 FROM task_relations r
                       JOIN tasks p ON p.task_id = r.to_task_id
                       WHERE r.from_task_id = t.task_id
                         AND r.relation_type = $3
                         AND p.sandbox_id = $2)))",
        )
        .bind(task_id)
        .bind(sandbox_id)
        .bind(TaskRelationType::CreatedBy.as_str())
        .fetch_one(&self.pool)
        .await
    }

    /// The Workspace `sandbox_id` materializes, or `None` for no such Sandbox.
    pub(super) async fn sandbox_workspace(
        &self,
        sandbox_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT workspace_id FROM sandboxes WHERE sandbox_id = $1")
            .bind(sandbox_id)
            .fetch_optional(&self.pool)
            .await
    }
}
