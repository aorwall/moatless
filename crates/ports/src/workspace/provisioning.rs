use async_trait::async_trait;
use moatless_types::store::StoreError;

/// The Workspace every Task belongs to, created on demand.
///
/// A Task names a Workspace or it names a repository. In the second case the
/// repository's deterministic shadow Workspace is what the Task is persisted
/// against, so every creation path stores a `workspace_id`. Idempotent: the
/// same repository always yields the same Workspace id.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait WorkspaceProvisioner: Send + Sync {
    async fn ensure_shadow(&self, repository_id: &str) -> Result<String, StoreError>;
}
