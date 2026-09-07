//! Task read authorization for binding commands.

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_ports::task::{TaskDescriptor, TaskReader};
use moatless_types::access_control::ScopeContext;

pub async fn ensure_task_access_with(
    tasks: &dyn TaskReader,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<TaskDescriptor, AdapterError> {
    tasks
        .describe(task_id, ctx)
        .await
        .map_err(|e| SharedAdapterError::Internal(e.to_string()))?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Task {task_id} not found")).into())
}

pub async fn ensure_task_access_only(
    tasks: &dyn TaskReader,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<(), AdapterError> {
    let allowed = tasks
        .may_access(task_id, ctx)
        .await
        .map_err(|e| SharedAdapterError::Internal(e.to_string()))?;
    if allowed {
        Ok(())
    } else {
        Err(SharedAdapterError::NotFound(format!("Task {task_id} not found")).into())
    }
}
