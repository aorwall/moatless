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
    /// must act on (no proxy domain, runtime doesn't publish ports, or an edge
    /// write failed costing only the URL). `Err` means the caller should not
    /// proceed.
    async fn publish_port(
        &self,
        task_id: &str,
        name: &str,
        port: u16,
    ) -> Result<Option<String>, SandboxError>;
}
