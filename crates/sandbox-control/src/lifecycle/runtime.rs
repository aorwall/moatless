//! The `SandboxRuntime` seam — the infrastructure-mutation interface lifecycle
//! owns.
//!
//! Every mutation returns a typed [`RuntimeOutcome`] so lifecycle never infers
//! what happened through a second status read, and takes a
//! [`RuntimeOperationContext`] so the operation generation reaches the runtime
//! and is stamped onto the resources it creates.
//!
//! The seam is held **only** by the lifecycle module: a task-scoped lifecycle
//! mutation that bypasses lifecycle is a compile error, not a review comment.
//! The one named exception is the orphan resource sweep (label-based deletion
//! of taskless resources), which lifecycle cannot express and which lives
//! outside this seam. The K8s and Docker adapters implement this trait; a
//! recording adapter implements it for lifecycle module tests.

use async_trait::async_trait;

use crate::error::SandboxError;

use super::error::RuntimeError;
use super::outcome::Diagnostics;
use super::types::{ArchiveRetention, ObservedStatus, OperationGeneration, RuntimeEffect};

/// Runtime-resource label carrying lifecycle's operation identity.
pub const OPERATION_GENERATION_LABEL: &str = "moatless.io/operation-generation";

/// Context threaded to **every** runtime mutation.
///
/// Carries the operation generation the adapter stamps onto the resources it
/// creates (K8s pod-template label, Docker container label) so that watcher
/// events can be correlated back to the operation that caused them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeOperationContext {
    pub operation_generation: OperationGeneration,
}

impl RuntimeOperationContext {
    pub fn new(operation_generation: OperationGeneration) -> Self {
        Self {
            operation_generation,
        }
    }

    pub fn generation_label(self) -> String {
        self.operation_generation.to_string()
    }
}

pub fn generation_from_label(value: Option<&String>) -> Option<OperationGeneration> {
    value?.parse::<i64>().ok().map(OperationGeneration)
}

/// Map infrastructure-facing adapter errors onto lifecycle's retry classes.
pub fn map_sandbox_error(error: SandboxError) -> RuntimeError {
    match error {
        SandboxError::BadRequest(message)
        | SandboxError::ConfigMissing(message)
        | SandboxError::PodBuildError(message)
        | SandboxError::Conflict(message)
        | SandboxError::Forbidden(message) => RuntimeError::Rejected(message),
        SandboxError::JwtError(message)
        | SandboxError::DockerError(message)
        | SandboxError::NotReady(message)
        | SandboxError::NotFound(message)
        | SandboxError::Internal(message) => RuntimeError::Infrastructure(message),
        SandboxError::KubeError(error) => RuntimeError::Infrastructure(error.to_string()),
        // Only the file-tree read produces this today, so no runtime mutation
        // can raise it; a malformed sandbox response is infrastructure-level.
        SandboxError::InvalidResponse(error) => RuntimeError::Infrastructure(error.to_string()),
        // Already a runtime failure that a provider helper raised — hand back
        // the original classification rather than flattening it to a string.
        SandboxError::Runtime(error) => error,
        // `RuntimeError` carries a message, not a source, so the projection is
        // lossy by construction: render the whole causal chain so nothing the
        // typed variants captured is dropped on the way to lifecycle.
        error @ (SandboxError::JwtSigning { .. }
        | SandboxError::Docker { .. }
        | SandboxError::DockerConfig(_)
        | SandboxError::Grpc { .. }
        | SandboxError::Unsupported { .. }
        | SandboxError::GrpcClient(_)
        | SandboxError::Database { .. }
        | SandboxError::Authorization(_)
        | SandboxError::Lifecycle(_)) => {
            RuntimeError::Infrastructure(moatless_core::error::causal_chain(&error))
        }
    }
}

/// Returned by **every** `SandboxRuntime` mutation.
///
/// Carries the observed status, the [`RuntimeEffect`] (so lifecycle knows
/// whether infrastructure actually changed), and success diagnostics. It never
/// describes failure — that is [`RuntimeError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOutcome {
    pub observed: ObservedStatus,
    pub effect: RuntimeEffect,
    pub diagnostics: Diagnostics,
}

impl RuntimeOutcome {
    pub fn applied(observed: ObservedStatus) -> Self {
        Self {
            observed,
            effect: RuntimeEffect::Applied,
            diagnostics: Diagnostics::default(),
        }
    }

    /// A recording-adapter builder: production adapters compute the effect from
    /// what they did rather than asserting `NoOp` up front.
    #[cfg(test)]
    pub fn no_op(observed: ObservedStatus) -> Self {
        Self {
            observed,
            effect: RuntimeEffect::NoOp,
            diagnostics: Diagnostics::default(),
        }
    }

    pub fn with_diagnostics(mut self, diagnostics: Diagnostics) -> Self {
        self.diagnostics = diagnostics;
        self
    }
}

/// The infrastructure mutation seam, implemented by the K8s and Docker
/// adapters (and, for module tests, a recording adapter).
///
/// `ensure_running` names what the runtime start actually does: reuse Ready,
/// wait for Initializing, scale up Stopped, rebuild on image change, create
/// missing, heal network resources, and replace a terminal/error pod from the
/// fresh plan. An existing failed pod is not reported as indefinitely
/// Initializing.
#[async_trait]
pub trait SandboxRuntime: Send + Sync {
    /// Ensure the sandbox is running from a fresh plan (start / reconcile
    /// converge / message-wake).
    async fn ensure_running(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;

    /// Stop compute, preserving storage. Idempotent against already-stopped
    /// compute (returns `NoOp`).
    async fn stop(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;

    /// Restart the current deployment, preserving identity by carrying the new
    /// generation onto the replacement. On Kubernetes this patches the
    /// pod-template under the `Recreate` strategy; on Docker it strictly
    /// removes and recreates the sandbox container (labels are immutable on a
    /// live container, so in-place restart could never carry identity).
    async fn restart(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;

    /// Replace the deployment from a fresh plan. **Edge teardown is strict**:
    /// a redeploy never proceeds to provision past a failed delete.
    async fn redeploy(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;

    /// Delete all runtime resources (Deployment, Service, IngressRoutes, and
    /// the PVC unconditionally), honoring `retention` for S3 and local-disk
    /// archives.
    ///
    /// **Strict success means observed absence, not accepted deletion**: the
    /// implementation awaits actual absence of every resource before returning
    /// `Ok`. An archive-deletion failure is load-bearing — it surfaces as a
    /// [`RuntimeError`], not a swallowed log.
    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;

    /// Reclaim the compute objects and the workspace volume of a **stopped**
    /// sandbox, keeping everything that gives the task its identity — the
    /// Service, the IngressRoutes, the network policy, and the S3 archive. A
    /// later start provisions a fresh volume and restores into it.
    ///
    /// The Deployment goes before the claim, and its absence is awaited: a
    /// pod template that still names a deleted claim leaves the next scale-up
    /// Pending forever.
    ///
    /// `rescue_upload_url` is `Some` when lifecycle could not prove the archive
    /// is current — the runtime must then get the volume's contents to that
    /// presigned URL **before** deleting anything, and fail the whole operation
    /// if it cannot. Lifecycle owns the proof because it owns the archive;
    /// the runtime owns how a detached volume is read.
    ///
    /// **Strict success means observed absence**, as with [`Self::remove`].
    /// Idempotent: a sandbox with no Deployment and no claim returns `NoOp`, so
    /// a caller that reruns on a schedule does no work on an already-cold task.
    async fn reclaim_storage(
        &self,
        task_id: &str,
        rescue_upload_url: Option<&str>,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::outcome::DegradationWarning;

    #[test]
    fn runtime_outcome_builders_set_effect() {
        assert_eq!(
            RuntimeOutcome::applied(ObservedStatus::Starting).effect,
            RuntimeEffect::Applied
        );
        assert_eq!(
            RuntimeOutcome::no_op(ObservedStatus::Ready).effect,
            RuntimeEffect::NoOp
        );
    }

    #[test]
    fn runtime_outcome_carries_degradations() {
        let diags = Diagnostics {
            degradations: vec![DegradationWarning {
                code: "storage_degraded".into(),
                message: "pvc failed, ephemeral fallback".into(),
            }],
            server_failures: vec![],
        };
        let outcome =
            RuntimeOutcome::applied(ObservedStatus::Starting).with_diagnostics(diags.clone());
        assert_eq!(outcome.diagnostics, diags);
    }

    #[test]
    fn context_threads_generation() {
        let ctx = RuntimeOperationContext::new(OperationGeneration(42));
        assert_eq!(ctx.operation_generation, OperationGeneration(42));
    }

    #[test]
    fn generation_label_round_trips() {
        let ctx = RuntimeOperationContext::new(OperationGeneration(42));
        let value = ctx.generation_label();
        assert_eq!(
            generation_from_label(Some(&value)),
            Some(OperationGeneration(42))
        );
        assert_eq!(generation_from_label(Some(&"invalid".to_string())), None);
    }
}
