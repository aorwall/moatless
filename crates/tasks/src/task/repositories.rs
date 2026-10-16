//! The repositories a Task works in: its Workspace's placements when it was
//! created, a fork's source Task's, and each one cloned while it runs.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use moatless_types::access_control::ScopeContext;
pub use moatless_types::task::TaskRepositorySource;

#[cfg(test)]
pub(crate) use super::repositories_dao::MockTaskRepositoryDao;
pub(crate) use super::repositories_dao::TaskRepositoryDao;

/// A row of `task_repositories`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskRepository {
    pub repository_id: String,
    #[sqlx(try_from = "String")]
    pub source: TaskRepositorySource,
    pub added_at: DateTime<Utc>,
}

/// The `task_repositories` table, as a host reads and writes it.
pub struct TaskRepositories {
    store: Arc<dyn TaskRepositoryDao>,
}

impl TaskRepositories {
    pub(crate) fn new(store: Arc<dyn TaskRepositoryDao>) -> Self {
        Self { store }
    }

    /// See [`TaskRepositoryDao::record`].
    pub async fn record(
        &self,
        task_id: &str,
        repository_ids: &[String],
        source: TaskRepositorySource,
    ) -> Result<(), sqlx::Error> {
        self.store.record(task_id, repository_ids, source).await
    }

    /// See [`TaskRepositoryDao::list`].
    pub async fn list(&self, task_id: &str) -> Result<Vec<TaskRepository>, sqlx::Error> {
        self.store.list(task_id).await
    }

    /// See [`TaskRepositoryDao::readable_in_workspace`].
    pub async fn readable_in_workspace(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Vec<String>, sqlx::Error> {
        self.store.readable_in_workspace(viewer, workspace_id).await
    }
}
