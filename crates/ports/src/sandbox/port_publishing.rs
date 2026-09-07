use async_trait::async_trait;
use moatless_types::sandbox::SandboxError;

/// Make one port on an already-running sandbox reachable from outside it.
///
/// Deliberately not a sixth verb on `SandboxRuntime`: that trait's verbs all
/// move a sandbox through its desired-state machine and carry an operation
/// generation. Publishing a port changes no desired state, bumps no generation,
/// and is not something the reconciler drives — a Script asks for it as it
/// starts.
///
/// Implementations must be idempotent: the same call for a port that is already
/// published succeeds and returns the same URL, and must refuse a reserved port
/// (`is_reserved_sandbox_port`) before doing anything else — see that function.
#[async_trait]
pub trait SandboxPortPublisher: Send + Sync {
    /// Publish `port`, returning the external URL when the deployment has one.
    ///
    /// `Ok(None)` means the port is not externally addressable but nothing went
    /// wrong that the caller must act on — no proxy domain is configured, the
    /// runtime does not publish ports at all, or an edge write failed in a way
    /// that costs only the URL. An `Err` means the caller should not proceed.
    async fn publish_port(
        &self,
        task_id: &str,
        name: &str,
        port: u16,
    ) -> Result<Option<String>, SandboxError>;
}
