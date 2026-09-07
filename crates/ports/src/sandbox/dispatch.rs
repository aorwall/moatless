use async_trait::async_trait;
use moatless_types::sandbox::SandboxError;
use moatless_types::task::CheckoutMode;

/// Registers a task as a member of an already-running sandbox process.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait SandboxJoiner: Send + Sync {
    async fn join_task(
        &self,
        task_id: &str,
        checkout_mode: CheckoutMode,
    ) -> Result<(), SandboxError>;
}

/// Ensure a task's sandbox is running and notify it of new work.
///
/// A closed task is a no-op. On failure the wake returns a human-readable
/// string so the caller can decide whether to surface it to the user.
#[async_trait]
pub trait SandboxWaker: Send + Sync {
    async fn wake(&self, task_id: &str) -> Result<(), String>;
}
