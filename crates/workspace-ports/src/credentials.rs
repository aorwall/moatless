use async_trait::async_trait;
use moatless_types::store::StoreError;
use std::collections::HashMap;

/// The provider credentials pinned to a Workspace, decrypted and keyed by
/// environment variable.
///
/// Takes no actor: the caller's authorization already decided the Workspace may
/// run. Unreadable credentials are an error — falling back would run under a
/// different identity than configured.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait WorkspaceCredentialReader: Send + Sync {
    async fn resolve_env(&self, workspace_id: &str) -> Result<HashMap<String, String>, StoreError>;
}
