//! Provider-agnostic interface for sandbox lifecycle operations.
//!
//! Implementations:
//! - `K8sSandboxProvider` (k8s/provider.rs) — Kubernetes-based sandboxes
//! - Future: ModalSandboxProvider, FlyIoSandboxProvider, etc.

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;

use super::error::SandboxError;
use super::types::{
    FileSearchResult, HealthResponse, ProvisionRequest, ProvisionResponse,
    ServerStatusResponse, StatusResponse, WorkspaceStatusFullResponse,
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

    /// Get full sandbox status (pod status + sandbox snapshot + proxy URLs).
    async fn sandbox_status(
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

    /// Get server status for all servers in the sandbox.
    async fn server_status(
        &self,
        task_id: &str,
    ) -> Result<ServerStatusResponse, SandboxError>;

    /// Stream server logs via SSE (proxied from sandbox pod).
    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError>;

    /// Search files in the sandbox workspace.
    async fn search_files(
        &self,
        task_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<FileSearchResult>, SandboxError>;
}
