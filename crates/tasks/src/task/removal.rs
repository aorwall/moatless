use crate::artifact::operations::Artifacts;

use super::bindings::{TaskBindings, deactivate_task_bindings};
use super::error::TaskError;
use super::models::TaskPatch;
use super::models::TaskStatus;
use super::operations::TaskCommands;
use moatless_ports::sandbox::SandboxLifecycle;
use moatless_types::access_control::ScopeContext;
use moatless_types::sandbox::ArchiveRetention;

/// Close membership, remove resources, and delete the row only after removal
/// succeeds.
pub async fn delete_task(
    sandbox: &dyn SandboxLifecycle,
    task_bindings: &dyn TaskBindings,
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope_ctx: &ScopeContext,
    task_id: String,
) -> Result<(), TaskError> {
    let row = tasks
        .find_by_id(&task_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    // Before the close below, which only needs write: a public Task's visitor
    // would otherwise close it and remove its Sandbox before `delete_task`'s
    // owner filter refused the row.
    if !scope_ctx.is_owner(&row.scoped_resource()) {
        return Err(TaskError::Forbidden(format!(
            "only the owner of task {task_id} can delete it"
        )));
    }

    // Leave the active member set before lifecycle decides whether this is a
    // member detach or the final sandbox-scoped remove. Concurrent deletes of
    // all members then serialize with at least one caller seeing no open/error
    // peers, so runtime resources are removed before the last row disappears.
    let closed = tasks
        .update_task(
            &task_id,
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                ..Default::default()
            },
            scope_ctx,
        )
        .await?;
    if !closed {
        return Err(TaskError::NotFound(task_id.clone()));
    }

    // Stop external listeners. `task_bindings.task_id` is not an FK, so rows
    // don't cascade on delete — deactivate them so inbound routing stops.
    deactivate_task_bindings(task_bindings, &task_id).await;

    // Permanent deletion is strict remove with archive deletion. The row is
    // deleted only on success; on failure it survives Closed with lifecycle
    // intent for the reconciler to finish.
    sandbox
        .remove(&task_id, ArchiveRetention::DeleteArchives)
        .await?;

    let deleted = tasks.delete_task(&task_id, scope_ctx).await?;
    if !deleted {
        return Err(TaskError::NotFound(task_id.clone()));
    }
    // After the row, so a publish racing this delete fails its reservation on
    // the missing Task rather than writing a page under a cleared prefix.
    artifacts.delete_task_pages(&task_id).await;
    tracing::info!(task_id = %task_id, user_id = %scope_ctx.user_id, "Task deleted");

    Ok(())
}
