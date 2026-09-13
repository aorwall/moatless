use super::dao::WorkspaceDao;
use super::error::WorkspaceError;
use super::git_sync::{GitSyncService, SyncForce};
use super::models::{Workspace, WorkspaceSource};
use crate::repository::dao::RepositoryDao;
use moatless_types::access_control::{ScopeContext, ScopedResource};
use tracing::warn;

pub(super) fn reset_source(ws: &Workspace) -> Result<(&str, &str), WorkspaceError> {
    match (ws.source_repository_id.as_deref(), ws.source_key.as_deref()) {
        (Some(repository_id), Some(source_key)) => Ok((repository_id, source_key)),
        _ => Err(WorkspaceError::BadRequest(
            "Workspace was not synced from Git; there is no config to reset to".to_string(),
        )),
    }
}

/// Reclaim the selected workspace from its Git provenance after checking
/// control.
pub async fn reset_git_workspace(
    workspaces: &dyn WorkspaceDao,
    repositories: &dyn RepositoryDao,
    git_sync: &GitSyncService,
    ctx: &ScopeContext,
    workspace_id: String,
) -> Result<Option<String>, WorkspaceError> {
    let existing = workspaces
        .find_by_id(&workspace_id, ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;

    // The sync writes with a system context, so control is checked here.
    let resource = ScopedResource::borrowed(
        existing.scope,
        existing
            .owner_user_id
            .as_deref()
            .or(existing.created_by.as_deref()),
    );
    if !ctx.can_write(&resource) {
        return Err(WorkspaceError::Forbidden(format!(
            "No control access to workspace '{workspace_id}'"
        )));
    }

    let (source_repository_id, source_key) = reset_source(&existing)?;
    // An already-git workspace has nothing to reclaim — for it the re-sync is
    // just a refresh, and success means the sync ran.
    let was_overridden = existing.source == WorkspaceSource::Manual;

    let repo = repositories
        .find_by_id(source_repository_id, ctx)
        .await
        .map_err(WorkspaceError::Db)?;
    let error = match repo {
        None => Some(format!(
            "Source repository '{source_repository_id}' not found or not accessible"
        )),
        Some(repo) => {
            let force = SyncForce::Only(workspace_id.clone());
            match git_sync.sync_repo_workspaces_for(&repo, ctx, &force).await {
                // `Only` reclaims at most this one workspace, so a non-zero
                // count is proof that *this* workspace was rewritten from git —
                // `updated` would also count the repo's other workspaces.
                Ok(summary) if !was_overridden || summary.reclaimed > 0 => None,
                Ok(_) => Some(format!(
                    "'{source_key}' is no longer declared in {}",
                    existing
                        .source_config_path
                        .as_deref()
                        .unwrap_or(".moatless/workspaces.json")
                )),
                Err(e) => {
                    warn!(%workspace_id, error = %e, "Reset-git re-sync failed");
                    Some(format!("Failed to read the config from Git: {e}"))
                }
            }
        }
    };

    Ok(error)
}
