//! Read-side and interior-connection Sandbox seams.
//!
//! Infrastructure mutations stay behind lifecycle's `SandboxRuntime`. HTTP
//! status handlers receive only [`SandboxObserver`], while file, notification,
//! credential-broadcast, and reaper callers receive only
//! [`SandboxConnector`].

use async_trait::async_trait;
use futures::stream::BoxStream;

use super::client::SandboxClient;
use super::error::SandboxError;
use super::live_status::LiveStatus;
use super::schemas::SandboxStatusFullResponse;
use crate::task::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};

/// Which slice of a server container's log to read.
#[derive(Debug, Clone, Copy)]
pub struct LogOptions {
    /// Read the previous (crashed/restarted) container instance instead of the
    /// live one. Implies no follow — that instance is gone.
    pub previous: bool,
    /// Hold the stream open and append lines as the container writes them.
    /// `false` returns what is already buffered and ends the stream.
    pub follow: bool,
    /// How many lines back from the end to start.
    pub tail_lines: i64,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            previous: false,
            follow: true,
            tail_lines: 500,
        }
    }
}

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

    /// One value per log line, and nothing about how a line is delivered.
    ///
    /// The SSE route frames each line at its own boundary and the T3 RPC
    /// producer sends it as a chunk value; an `axum` SSE `Event` is write-only
    /// and could not be re-emitted over the second transport.
    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        options: LogOptions,
    ) -> Result<BoxStream<'static, Result<String, SandboxError>>, SandboxError>;
}

/// The durable Sandbox intent recorded for a Task, as a read.
///
/// A status read has to be resolved against desired state — "stopped" and
/// "never asked for" are different answers — and `Lifecycle` is where that
/// state lives. This is the one thing a caller outside the Sandbox module
/// needs from it, so it is the only thing this seam offers: taking the whole
/// orchestrator would hand a read-only consumer every mutation on it, which is
/// the same reason [`SandboxObserver`] and [`SandboxConnector`] are separate.
#[async_trait]
pub trait SandboxIntentReader: Send + Sync {
    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxError>;
}

#[async_trait]
impl SandboxIntentReader for super::lifecycle::Lifecycle {
    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxError> {
        Ok(Self::desired_state(self, task_id).await?)
    }
}

/// Durable lifecycle write needed by transports that can start a stopped Task
/// environment without taking the whole sandbox API state.
#[async_trait]
pub trait SandboxStarter: Send + Sync {
    async fn start(&self, task_id: &str) -> Result<(), SandboxError>;
}

#[async_trait]
impl SandboxStarter for super::lifecycle::Lifecycle {
    async fn start(&self, task_id: &str) -> Result<(), SandboxError> {
        Self::start(self, task_id).await?;
        Ok(())
    }
}

/// Durable lifecycle write needed by transports that can stop a running Task
/// environment without taking the whole sandbox API state.
#[async_trait]
pub trait SandboxStopper: Send + Sync {
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxError>;
}

#[async_trait]
impl SandboxStopper for super::lifecycle::Lifecycle {
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxError> {
        Self::stop(self, task_id, reason).await?;
        Ok(())
    }
}

/// The only construction path for a task-addressed [`SandboxClient`].
///
/// The concrete Docker and Kubernetes adapters own their addressing rules and
/// readiness gate. Callers never construct service names or raw gRPC URLs.
#[async_trait]
pub trait SandboxConnector: Send + Sync {
    async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError>;
}
