//! Provider-agnostic interface for sandbox lifecycle operations.
//!
//! Implementations:
//! - `K8sSandboxProvider` (k8s/provider.rs) — Kubernetes-based sandboxes
//! - `DockerSandboxProvider` (docker/provider.rs) — Docker-based sandboxes

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;

use super::client::SandboxClient;
use super::error::SandboxError;
use super::live_status::LiveStatus;
use super::models::{ContainerObservedStatus, ProvisionRequest, ServerConfig};
use super::schemas::{ProvisionResponse, SandboxStatusFullResponse};
use crate::task::models::{SandboxDesiredState, TaskStatus};

#[async_trait]
pub trait SandboxProvider: Send + Sync {
    /// Provision a new sandbox or reuse an existing one.
    ///
    /// Idempotent: never destroys existing resources.
    /// - Running + ready → returns `Ready`
    /// - Exists but initializing/pending → returns `Initializing` (no recreate)
    /// - Stopped (scaled to 0) → scales up, returns `Initializing`
    /// - Nothing exists → creates resources, returns `Initializing`
    async fn provision(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError>;

    /// Get full sandbox status (pod status + sandbox snapshot + proxy URLs).
    ///
    /// `server_configs` are the canonical server definitions from the DB.
    /// Providers overlay runtime container statuses onto these configs.
    async fn sandbox_status(
        &self,
        task_id: &str,
        server_configs: &[ServerConfig],
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError>;

    /// Resolve the small per-Task live-status projection without diagnostics.
    async fn live_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<LiveStatus, SandboxError>;

    /// Image a fresh provision/redeploy would bake into the sandbox container:
    /// the per-repo `docker_image` override when non-empty, else the provider's
    /// configured default. Lets callers compare it against the running image to
    /// tell whether a redeploy would change the image.
    fn expected_sandbox_image(&self, repo_docker_image: Option<&str>) -> String;

    /// Stop a sandbox (preserve storage, scale to 0).
    async fn stop(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Restart a sandbox (delete pods, Deployment controller recreates).
    async fn restart(&self, task_id: &str) -> Result<(), SandboxError>;

    /// Redeploy a sandbox (delete Deployment/Service/IngressRoutes, recreate from fresh config).
    ///
    /// Preserves the PVC so workspace data is retained. Use this after updating
    /// repo server configs to apply the new configuration.
    async fn redeploy(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError>;

    /// Fully clean up a sandbox (remove all resources).
    ///
    /// When `retain_storage` is `true`, S3-backed archives (Claude transcript +
    /// `s3`-mode workspace) are preserved so the task can later be revived via
    /// reopen; K8s resources (Deployment/Service/PVC/IngressRoutes) are torn
    /// down regardless. When `false`, everything — including S3 archives — is
    /// removed (permanent deletion).
    async fn cleanup(&self, task_id: &str, retain_storage: bool) -> Result<(), SandboxError>;

    /// Fetch observed container statuses from the runtime (K8s pods / Docker).
    ///
    /// Returns raw container statuses (excluding the sandbox container itself).
    /// Handlers merge these with DB-sourced `ServerConfig`s to build the
    /// `ServerStatusResponse`.
    async fn container_statuses(
        &self,
        task_id: &str,
    ) -> Result<Vec<ContainerObservedStatus>, SandboxError>;

    /// Stream server logs via SSE (proxied from sandbox pod).
    ///
    /// When `previous` is `true`, fetches logs from the previous container
    /// instance (useful for crashed containers).
    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        previous: bool,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError>;

    /// Get a ready gRPC client for the sandbox identified by `task_id`.
    ///
    /// Verifies the sandbox is running before returning the client.
    /// Used by file operation handlers to call the sandbox directly.
    async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError>;
}
