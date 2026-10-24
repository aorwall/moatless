//! Workspace HTTP handlers.
//!
//! Read-only but for deleting a legacy manual Workspace: git is the only
//! writer, through `GitSyncService`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use moatless_core::http_auth::AuthUser;

use super::error::WorkspaceError;
use super::models::WorkspaceRepo;
use super::operations::Workspaces;
use super::schemas::{WorkspaceResponse, workspace_response};

/// Load a workspace (scoped) plus its placements into a response, or
/// `NotFound`.
async fn load_response(
    workspaces: &Workspaces,
    auth: &AuthUser,
    workspace_id: &str,
) -> Result<WorkspaceResponse, WorkspaceError> {
    let workspaces = workspaces.store();
    let ws = workspaces
        .find_by_id(workspace_id, &auth.scope_context())
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.to_string()))?;
    let repos = workspaces.find_repos(workspace_id).await?;
    Ok(workspace_response(ws, repos))
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces",
    tag = "workspaces",
    responses(
        (status = 200, description = "Workspaces", body = Vec<WorkspaceResponse>)
    )
)]
pub async fn list_workspaces(
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
) -> Result<Json<Vec<WorkspaceResponse>>, WorkspaceError> {
    let store = workspaces.store();
    let rows = store.list(&auth.scope_context()).await?;
    let ids: Vec<String> = rows.iter().map(|w| w.id.clone()).collect();
    let all_repos = store.find_repos_for_workspaces(&ids).await?;

    let responses: Vec<WorkspaceResponse> = rows
        .into_iter()
        .map(|w| {
            let repos: Vec<WorkspaceRepo> = all_repos
                .iter()
                .filter(|r| r.workspace_id == w.id)
                .cloned()
                .collect();
            workspace_response(w, repos)
        })
        .collect();
    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{workspace_id}",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    responses(
        (status = 200, description = "Workspace", body = WorkspaceResponse),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn get_workspace(
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    delete,
    path = "/api/v1/workspaces/{workspace_id}",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    responses(
        (status = 204, description = "Workspace deleted"),
        (status = 409, description = "Workspace is synced from Git", body = moatless_types::http::ErrorBody),
        (status = 403, description = "Forbidden", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn delete_workspace(
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<StatusCode, WorkspaceError> {
    super::mutations::delete_workspace(workspaces.store(), &auth.scope_context(), workspace_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
