//! Task repository — read-only queries for auto-provisioning.

use super::models::TaskRow;
use super::DbPool;

/// Find a task by its task_id.
pub async fn find_task_by_id(pool: &DbPool, task_id: &str) -> Result<Option<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>(
        "SELECT task_id, repository_id, branch, created_by FROM tasks WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
}
