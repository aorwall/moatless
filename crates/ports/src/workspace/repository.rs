use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_types::repository::Repository;
use moatless_types::store::StoreError;
use std::collections::HashMap;

/// The repository facts a peer band reads: one repository, or a batch of names.
///
/// [`Self::describe`] is scoped — it answers `None` for a repository the caller
/// may not be shown, exactly as one that does not exist. [`Self::names`] is not:
/// a caller batching names already holds ids it read through a scoped path, and
/// a name is not a fact worth a second walk.
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
