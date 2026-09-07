use async_trait::async_trait;
use moatless_types::store::StoreError;
use std::collections::HashMap;

/// The provider credentials pinned to a Workspace, decrypted and keyed by
/// environment variable.
///
/// Takes no actor: by the time these are read the caller's own authorization
/// already decided the Workspace may be run. A Workspace that holds credentials
/// whose values cannot be read is an error — falling back would run under a
/// different identity than the Workspace was configured with.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait WorkspaceCredentialReader: Send + Sync {
    async fn resolve_env(&self, workspace_id: &str) -> Result<HashMap<String, String>, StoreError>;
}
