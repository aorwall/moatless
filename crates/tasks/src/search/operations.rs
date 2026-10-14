//! The search reader.

use moatless_core::db::DbPool;
use moatless_types::access_control::ScopeContext;

use super::TaskMatch;
use super::dao;

/// Searches the open Tasks one viewer may read, followed or not.
#[derive(Clone)]
pub struct TaskSearch {
    pool: DbPool,
}

impl TaskSearch {
    pub fn new(pool: DbPool) -> Self {
        metrics::describe_counter!(
            dao::SEARCH_TIMEOUTS,
            "Task searches cancelled by their statement timeout"
        );
        Self { pool }
    }

    /// The open Tasks the viewer may read whose name or Message text contains
    /// `query`, at most one match each and `limit` in all.
    ///
    /// A person's Message beats the agent's, which beats the Task's name; ties
    /// go to the most recently updated Task. Subagent Messages and sandbox
    /// wakes are never matched.
    pub async fn search(
        &self,
        viewer: &ScopeContext,
        query: &str,
        limit: i64,
    ) -> Result<Vec<TaskMatch>, sqlx::Error> {
        dao::search(&self.pool, viewer, query, limit).await
    }
}
