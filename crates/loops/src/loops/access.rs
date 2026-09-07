//! Scoped Loop and repository access for Loop operations.

use super::dao::{LoopDao, LoopRecord};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_ports::workspace::RepositoryReader;
use moatless_types::access_control::{ScopeContext, ScopedResource, ensure_can_use_repository};

pub(crate) async fn ensure_repository_exists(
    repositories: &dyn RepositoryReader,
    ctx: &ScopeContext,
    repository_id: &str,
) -> Result<(), AdapterError> {
    let repo = repositories
        .describe(ctx, repository_id)
        .await?
        .ok_or_else(|| {
            SharedAdapterError::BadRequest(format!("Repository {repository_id} not found"))
        })?;
    ensure_can_use_repository(
        ctx,
        &ScopedResource::borrowed(
            repo.scope,
            repo.owner_user_id.as_deref().or(repo.created_by.as_deref()),
        ),
    )?;
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
