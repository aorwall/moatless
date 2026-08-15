//! Sandbox lifecycle status and commands for the T3 compatibility surface.
//!
//! This module owns only environment lifecycle answers: whether a Task's
//! Sandbox is available, and start/stop requests. Preview server declarations
//! and server logs live in `servers.rs`.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::data::{T3TaskDao, TaskView};
use moatless_backend_core::models::SandboxStopReason;
use moatless_backend_core::sandbox::{
    ClusterDiagnostics, SandboxError, SandboxIntentReader, SandboxObserver, SandboxStarter,
    SandboxStopper,
};
use moatless_backend_core::scope::ScopeContext;

/// Everything the sandbox lifecycle handlers need.
#[derive(Clone)]
pub struct SandboxState {
    pub observer: Arc<dyn SandboxObserver>,
    pub tasks: Arc<dyn T3TaskDao>,
    pub intent: Arc<dyn SandboxIntentReader>,
    pub starter: Arc<dyn SandboxStarter>,
    pub stopper: Arc<dyn SandboxStopper>,
    /// Fleet-wide status and live resource usage. Deployment-scoped rather than
    /// per-Task, so it takes no `tasks` authorization: the caller's environment
    /// scope is the whole of it.
    pub diagnostics: Arc<dyn ClusterDiagnostics>,
}

/// Why a sandbox lifecycle request could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum SandboxRpcError {
    /// The viewer cannot read this Task, or it does not exist.
    #[error("conversation not found")]
    NotFound,
    #[error("the sandbox could not be read")]
    Sandbox(#[source] SandboxError),
    #[error("the task could not be read")]
    Database(#[source] sqlx::Error),
}

async fn authorized_task(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<TaskView, SandboxRpcError> {
    state
        .tasks
        .find_by_id(task_id, viewer)
        .await
        .map_err(SandboxRpcError::Database)?
        .ok_or(SandboxRpcError::NotFound)
}

/// A Task's sandbox lifecycle status only, without declared server rows.
pub async fn status(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let task = authorized_task(state, viewer, task_id).await?;
    let desired = state
        .intent
        .desired_state(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    let status = state
        .observer
        .live_status(task_id, task.status, desired)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    Ok(json!({
        "sandboxStatus": status.sandbox_status.as_str(),
    }))
}

/// Request that a Task's Sandbox move toward Running, then return lifecycle
/// status only.
pub async fn start(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = authorized_task(state, viewer, task_id).await?;
    state
        .starter
        .start(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}

/// Every Sandbox in the deployment, with its status and live resource usage.
///
/// Deployment-global rather than per-Task: there is no `threadId` to authorize
/// against, so reaching this at all is the authorization, and the read is
/// already scoped to the caller's environment by the RPC dispatch.
pub async fn list_diagnostics(state: &SandboxState) -> Result<Value, SandboxRpcError> {
    let entries = state
        .diagnostics
        .list_sandbox_diagnostics()
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    Ok(json!({ "sandboxes": entries }))
}

/// Request that a Task's Sandbox move toward Stopped, then return lifecycle
/// status only.
pub async fn stop(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = authorized_task(state, viewer, task_id).await?;
    state
        .stopper
        .stop(task_id, SandboxStopReason::User)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}
