//! Read-side and interior-connection Sandbox seams.
//!
//! Infrastructure mutations stay behind lifecycle's `SandboxRuntime`. HTTP
//! status handlers receive only [`SandboxObserver`], while file, notification,
//! credential-broadcast, and reaper callers receive only
//! [`SandboxConnector`].

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;

use super::client::SandboxClient;
use super::error::SandboxError;
use super::live_status::LiveStatus;
use super::schemas::SandboxStatusFullResponse;
use crate::task::models::{SandboxDesiredState, TaskStatus};

/// Live runtime observation used by status and diagnostics routes.
#[async_trait]
pub trait SandboxObserver: Send + Sync {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError>;

    async fn live_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<LiveStatus, SandboxError>;

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        previous: bool,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError>;
}

/// The only construction path for a task-addressed [`SandboxClient`].
///
/// The concrete Docker and Kubernetes adapters own their addressing rules and
/// readiness gate. Callers never construct service names or raw gRPC URLs.
#[async_trait]
pub trait SandboxConnector: Send + Sync {
    async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError>;
}
