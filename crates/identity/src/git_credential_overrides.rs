use async_trait::async_trait;
use moatless_types::git_host::RepositoryProvider;
use moatless_types::store::StoreError;

/// Credentials that override a user's own git-host connection.
#[async_trait]
pub trait GitCredentialOverrides: Send + Sync {
    /// Return the selected credential, or fail if a configured override cannot be read.
    async fn token(
        &self,
        workspace_id: &str,
        provider: RepositoryProvider,
    ) -> Result<Option<String>, StoreError>;
}
