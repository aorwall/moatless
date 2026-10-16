//! `task_repositories` persistence.

use super::repositories::{TaskRepository, TaskRepositorySource};
use crate::task_listing::{LISTED_STATUSES, status_list};
use async_trait::async_trait;
use moatless_core::db::DbPool;
use moatless_types::access_control::{ScopeContext, ScopedEntity};
use sqlx::AssertSqlSafe;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait TaskRepositoryDao: Send + Sync {
    /// Add `repository_ids` to the Task. A repository it already has keeps
    /// its first `source`.
    async fn record(
        &self,
        task_id: &str,
        repository_ids: &[String],
        source: TaskRepositorySource,
    ) -> Result<(), sqlx::Error>;

    /// The Task's repositories, oldest first.
    async fn list(&self, task_id: &str) -> Result<Vec<TaskRepository>, sqlx::Error>;

    /// Every repository of a Task in `workspace_id` that `viewer` may read.
    async fn readable_in_workspace(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Vec<String>, sqlx::Error>;
}

pub(crate) struct PgTaskRepositoryDao(pub(crate) DbPool);

#[async_trait]
impl TaskRepositoryDao for PgTaskRepositoryDao {
    async fn record(
        &self,
        task_id: &str,
        repository_ids: &[String],
        source: TaskRepositorySource,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO task_repositories (task_id, repository_id, source)
               SELECT $1, repository_id, $3 FROM UNNEST($2::text[]) AS repository_id
               ON CONFLICT (task_id, repository_id) DO NOTHING"#,
        )
        .bind(task_id)
        .bind(repository_ids)
        .bind(source.as_str())
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list(&self, task_id: &str) -> Result<Vec<TaskRepository>, sqlx::Error> {
        sqlx::query_as::<_, TaskRepository>(
            "SELECT repository_id, source, added_at FROM task_repositories \
             WHERE task_id = $1 ORDER BY added_at, repository_id",
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }

    async fn readable_in_workspace(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Vec<String>, sqlx::Error> {
        // The listing's read rule, so a repository counts exactly when a Task
        // the viewer could open in the sidebar works in it.
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "t.", 2);
        let sql = format!(
            "SELECT DISTINCT tr.repository_id FROM task_repositories tr \
             JOIN tasks t ON t.task_id = tr.task_id \
             WHERE t.workspace_id = $1 AND t.status IN ({}) AND {}",
            status_list(LISTED_STATUSES),
            access.sql()
        );
        sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(workspace_id)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .fetch_all(&self.0)
            .await
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "repositories_dao_db_tests.rs"]
mod repositories_dao_db_tests;
