//! Sandbox error types.

/// Failure to stop an agent that is not running.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("Agent is not running")]
    AgentNotRunning,
}
