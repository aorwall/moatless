//! Repository config-sync handler (server-band composition).
//!
//! Moved here from `repository::handlers` because `sync_config` composes the
//! loop sync (`loops` band, `LoopSyncService`) with the git-declared workspace
//! sync (`workspaces` band) synchronously — naming `loops` from the
//! `repository` module would be an upward edge. See [`sync_repo_config`] for
//! how loop-sync and workspace-sync failures are handled differently.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use moatless_api_schemas::loops::{LoopSyncSummary, SyncConfigResponse};
use moatless_api_schemas::workspace::WorkspaceSyncSummary;

use moatless_core::http_auth::AuthUser;
use moatless_loops::loops::sync::LoopSyncService;
use moatless_workspaces::repository::dao::RepositoryDao;
use moatless_workspaces::repository::error::RepositoryError;
use moatless_workspaces::repository::handlers::log_workspace_sync;
use moatless_workspaces::repository::models::Repository;
use moatless_workspaces::workspace::git_sync::GitSyncService;
use moatless_workspaces::workspace::git_sync::SyncForce;

#[utoipa::path(
    post,
    path = "/api/v1/repositories/{repository_id}/sync-config",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "Config + loops synced", body = SyncConfigResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Repository not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn sync_config(
    State(repositories): State<Arc<dyn RepositoryDao>>,
    State(loop_sync): State<Arc<LoopSyncService>>,
    State(git_sync): State<Arc<GitSyncService>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<SyncConfigResponse>, RepositoryError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(RepositoryError::Forbidden(
            "Only administrators can synchronize repository configuration".to_string(),
        ));
    }
    let scope_ctx = auth.scope_context();
    let repo = repositories
        .find_by_id(&repository_id, &scope_ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    // Explicit, user-initiated sync: git is authoritative again, so overridden
    // workspaces are reclaimed. The automatic sync on task start is not.
    let (updated, loops, workspaces) =
        sync_repo_config(&loop_sync, &git_sync, &repo, &auth, &SyncForce::All).await?;

    Ok(Json(SyncConfigResponse {
        repository: moatless_workspaces::repository::schemas::repository_response(updated),
        loops,
        workspaces,
    }))
}

/// Reconcile a repository's committed `.moatless/loops/` + git-declared
/// workspaces into the loop tables and `workspaces` rows.
///
/// Run-config from `.moatless/config.json` / `.moatless/workspaces.json` flows
/// into the workspace (via `sync_repo_workspaces`), not into repo columns — a
/// repository is only a git connection. Loop sync failures abort the explicit
/// operation so the administrator cannot mistake a skipped refresh for a
/// successful approval revocation. Workspace sync failures do not: calling
/// [`log_workspace_sync`] instead of `sync_repo_workspaces` directly swallows
/// them to a warning and an empty summary, so a workspace hiccup never blocks
/// the same approval-revocation sync.
async fn sync_repo_config(
    loop_sync: &Arc<LoopSyncService>,
    git_sync: &Arc<GitSyncService>,
    repo: &Repository,
    auth: &AuthUser,
    force: &SyncForce,
) -> Result<(Repository, LoopSyncSummary, WorkspaceSyncSummary), RepositoryError> {
    if repo.remote_url.is_none() {
        return Err(RepositoryError::BadRequest(
            "Repository has no remote URL".to_string(),
        ));
    }

    // This is the sole repository-wide Loop sync path. Surface fetch and
    // reconciliation failures so an administrator never mistakes a skipped
    // refresh for a successful approval revocation.
    let loops = loop_sync
        .sync_repo_loops(repo, auth)
        .await
        .map_err(|error| RepositoryError::ProviderError(error.to_string()))?;

    let workspaces = log_workspace_sync(git_sync, repo, auth, "sync-config", force).await;

    Ok((repo.clone(), loops, workspaces))
}
