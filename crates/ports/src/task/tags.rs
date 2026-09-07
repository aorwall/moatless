use async_trait::async_trait;
use moatless_types::store::StoreError;

/// Tag names to tag ids.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TagResolver: Send + Sync {
    /// Ids for `names`, creating any tag that does not exist yet.
    async fn resolve(&self, names: &[String]) -> Result<Vec<String>, StoreError>;
}
