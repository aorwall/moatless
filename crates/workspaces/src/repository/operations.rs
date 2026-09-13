//! The repository capability a host holds.

use std::sync::Arc;

use moatless_types::access_control::ScopeContext;

use super::dao::RepositoryDao;
use super::models::Repository;

/// The `repositories` table, as a host and this crate's own routes reach it.
///
/// The store stays private to this crate; a peer band that only reads a
/// repository takes [`moatless_workspace_ports::RepositoryReader`] instead.
pub struct Repositories {
    store: Arc<dyn RepositoryDao>,
}

impl Repositories {
    pub(crate) fn new(store: Arc<dyn RepositoryDao>) -> Self {
        Self { store }
    }

    /// The store, for this crate's own handlers and mutations.
    pub(crate) fn store(&self) -> &dyn RepositoryDao {
        self.store.as_ref()
    }

    /// One repository the caller may read, or `None`.
    pub async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Repository>, sqlx::Error> {
        self.store.find_by_id(id, ctx).await
    }
}
