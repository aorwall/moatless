//! Repository config-sync handler (server-band composition).
//!
//! Moved here from `repository::handlers` because `sync_config` composes the
//! loop sync (`loops` band, `LoopSyncService`) with the git-declared workspace
//! sync (`workspaces` band) synchronously — naming `loops` from the
//! `repository` module would be an upward edge. The workspace half deliberately
//! degrades to an empty summary on failure (`log_workspace_sync`), while the
//! loop half surfaces failure to the admin; that asymmetry is preserved by
//! calling the same `log_workspace_sync` rather than `sync_repo_workspaces`
//! directly.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;

use moatless_api_schemas::loops::{LoopSyncSummary, SyncConfigResponse};
use moatless_api_schemas::workspace::WorkspaceSyncSummary;

use crate::extractors::AuthUser;
use crate::loops::sync::LoopSyncService;
use crate::repository::dao::RepositoryDao;
use crate::repository::error::RepositoryError;
use crate::repository::handlers::log_workspace_sync;
use crate::repository::models::Repository;
use crate::workspace::git_sync::{GitSyncService, SyncForce};

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
/// successful approval revocation.
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

    // Reconcile git-declared workspaces (`.moatless/workspaces.json` or the
    // legacy `.moatless/config.json` → workspace run-config). `log_workspace_sync`
    // swallows a workspace-sync failure into an empty summary + a warn, so a
    // workspace hiccup never fails the admin's approval-revocation sync — the
    // deliberate asymmetry with the loop half above.
    let workspaces = log_workspace_sync(git_sync, repo, auth, "sync-config", force).await;

    Ok((repo.clone(), loops, workspaces))
}
