use async_trait::async_trait;
use moatless_types::sandbox::SandboxError;

/// Make one port on an already-running sandbox reachable from outside it.
///
/// Not a `SandboxRuntime` verb: publishing changes no desired state and bumps
/// no generation. Implementations must be idempotent and must refuse a reserved
/// port (`is_reserved_sandbox_port`) first.
#[async_trait]
pub trait SandboxPortPublisher: Send + Sync {
    /// Publish `port`, returning the external URL when the deployment has one.
    ///
    /// `Ok(None)` means not externally addressable but not an error the caller
    /// must act on (no proxy domain, or runtime doesn't publish ports). `Err`
    /// means the caller should not proceed, including a failed edge write: a
    /// URL without its route answers 404.
    async fn publish_port(
        &self,
        task_id: &str,
        name: &str,
        port: u16,
    ) -> Result<Option<String>, SandboxError>;
}
