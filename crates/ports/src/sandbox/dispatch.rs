use async_trait::async_trait;
use moatless_types::agent::AgentProfile;
use moatless_types::sandbox::SandboxError;
use moatless_types::task::CheckoutMode;

/// Registers a task as a member of an already-running sandbox process.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait SandboxJoiner: Send + Sync {
    /// `agent_profile` is the joining Task's own harness selection. The pod boots
    /// with one Member's `AGENT_PROFILE` in its environment, so a Member that
    /// joins afterwards has to carry its own or run under the wrong CLI.
    async fn join_task(
        &self,
        task_id: &str,
        checkout_mode: CheckoutMode,
        agent_profile: &AgentProfile,
    ) -> Result<(), SandboxError>;
}

/// Ensure a task's sandbox is running and notify it of new work.
///
/// A closed task is a no-op; on failure the returned string is meant for the
/// user.
#[async_trait]
pub trait SandboxWaker: Send + Sync {
    async fn wake(&self, task_id: &str) -> Result<(), String>;
}
