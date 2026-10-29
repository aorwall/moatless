use async_trait::async_trait;

/// Hands a task's Sandbox to the person now acting in it: its credentials
/// resolve for them and its task tokens are minted for them from here on.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait SandboxDriver: Send + Sync {
    /// Make `user_id` the driver of `task_id`'s Sandbox, revoking every token
    /// minted for the previous one. A no-op when they already drive it, and
    /// when the task has no Sandbox row yet: provisioning reads the driver.
    async fn drive(&self, task_id: &str, user_id: &str) -> Result<(), DriveError>;
}

/// Why [`SandboxDriver::drive`] did not hand the Sandbox over.
#[derive(Debug, thiserror::Error)]
pub enum DriveError {
    /// `user_id` cannot write the task, so may not hold its credentials.
    #[error("{0}")]
    Forbidden(String),
    /// The switch, or its push to the running pod, failed.
    #[error("{0}")]
    Unavailable(String),
}
