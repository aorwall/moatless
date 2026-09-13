use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;
use moatless_workspace_types::repository::Repository;
use std::collections::HashMap;

/// The repository facts a peer band reads: one repository, or a batch of names.
///
/// [`Self::describe`] is scoped — a repository the caller may not see reads as
/// `None`, same as one that does not exist; [`Self::names`] is not, since its
/// ids already came from a scoped read.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait RepositoryReader: Send + Sync {
    /// The repository `ctx` may see, or `None` when there is none to show.
    async fn describe(
        &self,
        ctx: &ScopeContext,
        repository_id: &str,
    ) -> Result<Option<Repository>, StoreError>;

    /// Display name per repository id. Ids with no repository are absent.
    async fn names(&self, repository_ids: &[String])
    -> Result<HashMap<String, String>, StoreError>;
}
