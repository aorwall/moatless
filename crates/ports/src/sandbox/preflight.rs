use async_trait::async_trait;
use moatless_types::task::Task;
use moatless_workspace_types::repository::Repository;

/// Preflight seam: credential validation before a sandbox is provisioned.
///
/// Wired once in state assembly (`RealPreflight`) so every creation path gets
/// the same check for free; mocked in unit tests.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait PreflightPort: Send + Sync {
    async fn validate(&self, task: &Task, repo: &Repository) -> Result<(), PreflightRejection>;
}

/// Band-clean preflight failure returned by [`PreflightPort`].
///
/// Pre-computed from the rich `PreflightError` so the tasks band names no
/// preflight-credential type: `user_message` drives the 403 body +
/// `sandbox.error` event; `detail` is the underlying `Display` for logs.
#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct PreflightRejection {
    pub user_message: String,
    pub detail: String,
}
