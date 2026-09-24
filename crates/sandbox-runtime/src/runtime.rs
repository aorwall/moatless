//! The `SandboxRuntime` seam — the infrastructure-mutation interface lifecycle
//! owns.
//!
//! Every mutation returns a typed [`RuntimeOutcome`] (no inferring via a second
//! status read) and takes a [`RuntimeOperationContext`].
//!
//! Held only by the lifecycle module — a task-scoped mutation that bypasses
//! lifecycle is a compile error. The one exception is the orphan resource sweep
//! (label-based deletion of taskless resources), which lifecycle cannot
//! express.

use async_trait::async_trait;

use thiserror::Error;

use crate::error::SandboxError;

use crate::operation::{ObservedStatus, OperationGeneration, RuntimeEffect};
use crate::outcome::Diagnostics;
use moatless_types::sandbox::ArchiveRetention;

/// Runtime-resource label carrying lifecycle's operation identity.
pub const OPERATION_GENERATION_LABEL: &str = "moatless.io/operation-generation";

/// Context threaded to every runtime mutation.
///
/// Carries the operation generation the adapter stamps onto the resources it
/// creates (K8s pod-template label, Docker container label), so watcher events
/// can be correlated back to the operation that caused them.
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
        | SandboxError::Store(_)
        | SandboxError::Authorization(_)
        | SandboxError::Lifecycle(_)) => {
            RuntimeError::Infrastructure(moatless_core::error::causal_chain(&error))
        }
    }
}

/// Returned by every `SandboxRuntime` mutation.
///
/// Carries observed status, [`RuntimeEffect`] (did infrastructure change), and
/// success diagnostics; never describes failure — that is [`RuntimeError`].
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
    // `test` stays first so the census keeps scoring this as test-context; the
    // feature exposes it to the orchestrator band's tests.
    #[cfg(any(test, feature = "test-support"))]
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

/// The infrastructure mutation seam, implemented by the K8s and Docker adapters
/// (and, for module tests, a recording adapter).
///
/// `ensure_running` covers reuse-Ready, wait-Initializing, scale-up-Stopped,
/// rebuild-on-image-change, create-missing, heal-network, and
/// replace-terminal-pod — a failed pod is never reported as indefinitely
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
    /// generation onto the replacement.
    ///
    /// Kubernetes patches the pod-template under `Recreate`; Docker strictly
    /// removes and recreates the container, since labels are immutable on a
    /// live container.
    async fn restart(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;

    /// Replace the deployment from a fresh plan. **Edge teardown is strict**: a
    /// redeploy never proceeds to provision past a failed delete.
    async fn redeploy(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;

    /// Delete all runtime resources (Deployment, Service, `IngressRoutes`, PVC
    /// unconditionally), honoring `retention` for S3 and local-disk archives.
    ///
    /// Strict success means observed absence, not accepted deletion: awaits
    /// actual absence before returning `Ok`; an archive-deletion failure
    /// surfaces as a [`RuntimeError`], never a swallowed log.
    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;

    /// Reclaim the compute objects and workspace volume of a **stopped**
    /// sandbox, keeping the Service, `IngressRoutes`, network policy, and S3
    /// archive; a later start provisions a fresh volume and restores into it.
    ///
    /// The Deployment's absence is awaited before the claim is deleted (else a
    /// stale pod template leaves the next scale-up Pending forever);
    /// `rescue_upload_url`, when `Some`, must be filled before deleting
    /// anything or the operation fails. Strict success means observed absence,
    /// and it's idempotent (`NoOp` when nothing remains).
    async fn reclaim_storage(
        &self,
        task_id: &str,
        rescue_upload_url: Option<&str>,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError>;
}

/// Coarse error class driving retry policy.
///
/// The reconciler retries only *transient* classes (runtime/infra) under its
/// attempt cap, never *permanent* ones (the Closed guard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient infrastructure/runtime failure — safe to retry with backoff.
    Transient,
    /// Permanent failure — retrying cannot help (bad plan, Closed guard,
    /// missing task). The reconciler must not spin on these.
    Permanent,
    /// Per-Task lock contention — not a failure of the operation itself;
    /// resolved by serialization, not by the attempt cap.
    Contention,
}

impl ErrorClass {
    pub fn is_transient(self) -> bool {
        matches!(self, Self::Transient)
    }
}

/// Failure raised by a [`SandboxRuntime`] mutation.
///
/// Failure diagnostics belong here, never on the success [`RuntimeOutcome`];
/// adapters have no event publisher, so they surface everything through this
/// type and lifecycle republishes it.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// A transient infrastructure failure (API timeout, conflict, network).
    #[error("runtime infrastructure error: {0}")]
    Infrastructure(String),

    /// The runtime rejected the request permanently (e.g. an unbuildable plan).
    /// Retrying cannot help.
    #[error("runtime rejected the request: {0}")]
    Rejected(String),

    /// A delete was accepted but the resource did not reach observed absence
    /// within the wait bound. Strict removal treats accepted-but-not-absent as
    /// **not** success, so this is the failure a stuck deletion surfaces.
    #[error("resource still present after deletion was accepted: {0}")]
    NotAbsent(String),
}

impl RuntimeError {
    pub fn class(&self) -> ErrorClass {
        match self {
            // A stuck-but-accepted deletion is retriable — the reconciler
            // re-drives remove and re-awaits absence.
            Self::Infrastructure(_) | Self::NotAbsent(_) => ErrorClass::Transient,
            Self::Rejected(_) => ErrorClass::Permanent,
        }
    }

    /// Bounded cause slug published as the `Error` fact's `lifecycle_event`.
    /// Bounded because it becomes a UI label key and a log correlation field.
    pub fn cause(&self) -> &'static str {
        match self {
            Self::Infrastructure(_) => "runtime_infrastructure",
            Self::Rejected(_) => "runtime_rejected",
            Self::NotAbsent(_) => "runtime_not_absent",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::DegradationWarning;

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
