//! Scoped Loop and workspace access for Loop operations.

use super::dao::{LoopDao, LoopRecord};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_ports::workspace::WorkspaceResolver;
use moatless_types::access_control::ScopeContext;

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

pub(crate) async fn load_loop_for_auth_with(
    loops: &LoopDao,
    ctx: &ScopeContext,
    id: &str,
) -> Result<LoopRecord, AdapterError> {
    loops
        .get(id, ctx)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")).into())
}
