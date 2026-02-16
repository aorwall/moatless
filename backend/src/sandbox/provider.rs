//! Provider-agnostic interface for sandbox lifecycle operations.
//!
//! Implementations:
//! - `K8sSandboxProvider` (k8s/provider.rs) — Kubernetes-based sandboxes
//! - Future: ModalSandboxProvider, FlyIoSandboxProvider, etc.

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;

use super::error::SandboxError;

/// Whether `start()` found an already-running sandbox or initiated startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartResult {
    /// Sandbox was already running (Deployment exists at replicas>=1 with ready pods).
    AlreadyRunning,
    /// Sandbox is starting up (scaled from 0, or auto-provisioned).
    Starting,
}
use super::models::ProvisionRequest;
use super::schemas::{
    FileContentResponse, FileSearchResult, FileTreeResponse, HealthResponse, ListFilesResponse,
    ProvisionResponse, SearchByNameResponse, ServerStatusResponse, StatusResponse,
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

    /// Get full sandbox status (pod status + sandbox snapshot + proxy URLs).
    async fn sandbox_status(
        &self,
        task_id: &str,
    ) -> Result<WorkspaceStatusFullResponse, SandboxError>;

    /// Stop a sandbox (preserve storage, scale to 0).
    async fn stop(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Start a stopped sandbox (scale to 1).
    ///
    /// Returns `AlreadyRunning` if the sandbox was already up, or `Starting`
    /// if it needed to be scaled up or auto-provisioned.
    async fn start(&self, task_id: &str) -> Result<StartResult, SandboxError>;

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

    // ── File operations ──────────────────────────────────────────

    /// Get the file tree with git change overlay.
    async fn get_file_tree(
        &self,
        task_id: &str,
        root_path: &str,
    ) -> Result<FileTreeResponse, SandboxError>;

    /// Read a file from the sandbox workspace.
    async fn read_file(
        &self,
        task_id: &str,
        path: &str,
        include_changes: bool,
    ) -> Result<FileContentResponse, SandboxError>;

    /// Write a file in the sandbox workspace.
    async fn write_file(
        &self,
        task_id: &str,
        path: &str,
        content: &str,
    ) -> Result<(), SandboxError>;

    /// List files in the sandbox workspace.
    async fn list_files(
        &self,
        task_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<ListFilesResponse, SandboxError>;

    /// Search files by name substring in the sandbox workspace.
    async fn search_files_by_name(
        &self,
        task_id: &str,
        query: &str,
        case_sensitive: bool,
        max_results: i32,
    ) -> Result<SearchByNameResponse, SandboxError>;
}
