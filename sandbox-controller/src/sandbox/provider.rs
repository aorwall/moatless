//! Provider-agnostic interface for sandbox lifecycle operations.
//!
//! Implementations:
//! - `K8sSandboxProvider` (k8s/provider.rs) — Kubernetes-based sandboxes
//! - Future: ModalSandboxProvider, FlyIoSandboxProvider, etc.

use async_trait::async_trait;

use super::error::SandboxError;
use super::types::{
    HealthResponse, ProvisionRequest, ProvisionResponse, StatusResponse,
    WorkspaceStatusFullResponse,
};

#[async_trait]
pub trait SandboxProvider: Send + Sync {
    /// Provision a new sandbox or reuse an existing one.
    async fn provision(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError>;

    /// Get the current status of a sandbox.
    async fn status(&self, task_id: &str) -> Result<StatusResponse, SandboxError>;

    /// Check the health of a sandbox.
    async fn health(&self, task_id: &str) -> HealthResponse;

    /// Get full workspace status (pod status + sandbox snapshot + proxy URLs).
    async fn workspace_status(
        &self,
        task_id: &str,
    ) -> Result<WorkspaceStatusFullResponse, SandboxError>;

    /// Stop a sandbox (preserve storage, scale to 0).
    async fn stop(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Start a stopped sandbox (scale to 1).
    async fn start(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Restart a sandbox (delete pods, Deployment controller recreates).
    async fn restart(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Fully clean up a sandbox (remove all resources).
    async fn cleanup(&self, task_id: &str) -> Result<(), SandboxError>;
}
