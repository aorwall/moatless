//! Scoped Loop and workspace access for Loop operations.

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::access_control::ScopeContext;
use moatless_workspace_ports::WorkspaceResolver;

/// A Workspace `ctx` may run a Loop in. `describe` is scoped, so one the caller
/// cannot see reads as one that does not exist.
pub(crate) async fn ensure_workspace_usable(
    workspaces: &dyn WorkspaceResolver,
    ctx: &ScopeContext,
    workspace_id: &str,
) -> Result<(), AdapterError> {
    let workspace = workspaces
        .describe(ctx, workspace_id)
        .await?
        .ok_or_else(|| {
            SharedAdapterError::BadRequest(format!("Workspace {workspace_id} not found"))
        })?;
    if workspace.deleted {
        return Err(SharedAdapterError::BadRequest(format!(
            "Workspace {workspace_id} has been deleted"
        ))
        .into());
    }
    Ok(())
}
