//! The two status reads every container runtime answers the same way.
//!
//! A runtime's `SandboxStatusReader` impl differs only in `sandbox_logs`. Both
//! reads below resolve the sandbox id from the plan, then hand it to the
//! runtime's own resolver through [`StatusResolve`].

use async_trait::async_trait;

use crate::lifecycle::ProvisionDeps;
use moatless_sandbox_runtime::client::SandboxConnector;
use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::live_status::LiveStatus;
use moatless_sandbox_runtime::schemas::SandboxStatusFullResponse;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskStatus;

/// A container runtime's own status resolver, as the shared reads call it.
///
/// `connector` is the adapter that owns the resolver — a resolver reaches the
/// running sandbox over gRPC through it rather than holding its own client.
#[async_trait]
pub trait StatusResolve: Send + Sync {
    /// Full sandbox status: the environment, its agent and its diagnostics.
    async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> SandboxStatusFullResponse;

    /// Only the Task's live status, omitting diagnostics.
    async fn resolve_live_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> LiveStatus;
}

/// The sandbox id carrying a Task's containers.
///
/// A deployment with no plan wired runs one sandbox per Task under the Task's
/// own id, so `None` is not an error.
pub async fn sandbox_id_for_task(
    plan: Option<&ProvisionDeps>,
    task_id: &str,
) -> Result<String, SandboxError> {
    match plan {
        Some(plan) => plan
            .sandbox_id_for_task(task_id)
            .await
            .map_err(SandboxError::Runtime),
        None => Ok(task_id.to_string()),
    }
}

/// Full status: the sandbox, its agent and its diagnostics.
pub async fn sandbox_status(
    resolver: &dyn StatusResolve,
    plan: Option<&ProvisionDeps>,
    connector: &dyn SandboxConnector,
    task_id: &str,
    task_status: TaskStatus,
    desired_state: SandboxDesiredState,
) -> Result<SandboxStatusFullResponse, SandboxError> {
    let sandbox_id = sandbox_id_for_task(plan, task_id).await?;
    Ok(resolver
        .resolve_sandbox_status(task_id, &sandbox_id, task_status, desired_state, connector)
        .await)
}

/// Display status only.
pub async fn live_status(
    resolver: &dyn StatusResolve,
    plan: Option<&ProvisionDeps>,
    connector: &dyn SandboxConnector,
    task_id: &str,
    task_status: TaskStatus,
    desired_state: SandboxDesiredState,
) -> Result<LiveStatus, SandboxError> {
    let sandbox_id = sandbox_id_for_task(plan, task_id).await?;
    Ok(resolver
        .resolve_live_status(task_id, &sandbox_id, task_status, desired_state, connector)
        .await)
}
