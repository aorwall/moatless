//! Repository configuration synchronization HTTP adaptation.

use axum::{
    Json,
    extract::{Path, State},
};
use moatless_api_schemas::loops::SyncConfigResponse;
use moatless_core::http_auth::AuthUser;
use moatless_loops::loops::sync::LoopSyncService;
use moatless_workspaces::repository::{dao::RepositoryDao, error::RepositoryError};
use moatless_workspaces::workspace::git_sync::GitSyncService;
use std::sync::Arc;

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
    let result = super::sync::sync_config(
        repositories.as_ref(),
        loop_sync.as_ref(),
        git_sync.as_ref(),
        &auth.scope_context(),
        &repository_id,
    )
    .await?;
    Ok(Json(SyncConfigResponse {
        repository: moatless_workspaces::repository::schemas::repository_response(
            result.repository,
        ),
        loops: result.loops,
        workspaces: result.workspaces,
    }))
}
