//! Workspace HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::repository::Repositories;
use moatless_core::http_auth::AuthUser;
use moatless_ports::identity::UserResolver;

use super::error::WorkspaceError;
use super::git_sync::GitSyncService;
use super::models::WorkspaceRepo;
use super::operations::Workspaces;
use super::schemas::{
    AddRepoRequest, CreateWorkspaceRequest, ResetGitWorkspaceResponse, UpdateWorkspaceRequest,
    WorkspaceResponse, workspace_response,
};

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
    post,
    path = "/api/v1/workspaces",
    tag = "workspaces",
    request_body = CreateWorkspaceRequest,
    responses(
        (status = 201, description = "Workspace created", body = WorkspaceResponse),
        (status = 400, description = "Invalid request", body = moatless_types::http::ErrorBody),
        (status = 403, description = "Forbidden", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn create_workspace(
    State(workspaces): State<Arc<Workspaces>>,
    State(repositories): State<Arc<Repositories>>,
    State(users): State<Arc<dyn UserResolver>>,
    auth: AuthUser,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceResponse>), WorkspaceError> {
    let workspace_id = super::mutations::create_workspace(
        workspaces.store(),
        repositories.store(),
        users.as_ref(),
        &auth.scope_context(),
        req,
    )
    .await?;
    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok((StatusCode::CREATED, Json(response)))
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
    patch,
    path = "/api/v1/workspaces/{workspace_id}",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    request_body = UpdateWorkspaceRequest,
    responses(
        (status = 200, description = "Workspace updated", body = WorkspaceResponse),
        (status = 403, description = "Forbidden", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn update_workspace(
    State(workspaces): State<Arc<Workspaces>>,
    State(repositories): State<Arc<Repositories>>,
    State(users): State<Arc<dyn UserResolver>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    super::mutations::update_workspace(
        workspaces.store(),
        repositories.store(),
        users.as_ref(),
        &auth.scope_context(),
        workspace_id.clone(),
        req,
    )
    .await?;
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

#[utoipa::path(
    patch,
    path = "/api/v1/workspaces/{workspace_id}/override",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    request_body = UpdateWorkspaceRequest,
    responses(
        (status = 200, description = "Workspace overridden; source set to manual", body = WorkspaceResponse),
        (status = 403, description = "Forbidden", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody)
    )
)]
/// Override a git-synced workspace. Sets `source = "manual"` so that git sync
/// skips this workspace going forward. Provenance fields are preserved. Use
/// `POST /{id}/reset-git` to re-enable git sync.
pub async fn override_workspace(
    State(workspaces): State<Arc<Workspaces>>,
    State(users): State<Arc<dyn UserResolver>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    super::mutations::override_workspace(
        workspaces.store(),
        users.as_ref(),
        &auth.scope_context(),
        workspace_id.clone(),
        req,
    )
    .await?;
    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/reset-git",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    responses(
        (status = 200, description = "Re-sync attempted; `resynced` reports whether it took effect", body = ResetGitWorkspaceResponse),
        (status = 400, description = "Workspace was never synced from git", body = moatless_types::http::ErrorBody),
        (status = 403, description = "Forbidden", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody)
    )
)]
/// Discard a workspace's local override and take its run-config from git again.
///
/// Implemented as a re-sync of just this workspace, not a flag flip, so
/// `source` only returns to `"git"` when the repository still declares the
/// entry — never claiming git provenance while holding config git never
/// produced.
pub async fn reset_git_workspace(
    State(workspaces): State<Arc<Workspaces>>,
    State(repositories): State<Arc<Repositories>>,
    State(git_sync): State<Arc<GitSyncService>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<Json<ResetGitWorkspaceResponse>, WorkspaceError> {
    let error = super::reset::reset_git_workspace(
        workspaces.store(),
        repositories.store(),
        &git_sync,
        &auth.scope_context(),
        workspace_id.clone(),
    )
    .await?;
    let workspace = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(ResetGitWorkspaceResponse {
        resynced: error.is_none(),
        error,
        workspace,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/repos",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    request_body = AddRepoRequest,
    responses(
        (status = 200, description = "Repo added", body = WorkspaceResponse),
        (status = 400, description = "Invalid request", body = moatless_types::http::ErrorBody),
        (status = 403, description = "Forbidden", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn add_workspace_repo(
    State(workspaces): State<Arc<Workspaces>>,
    State(repositories): State<Arc<Repositories>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
    Json(req): Json<AddRepoRequest>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    super::mutations::add_workspace_repo(
        workspaces.store(),
        repositories.store(),
        &auth.scope_context(),
        workspace_id.clone(),
        req,
    )
    .await?;
    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    delete,
    path = "/api/v1/workspaces/{workspace_id}/repos/{placement_id}",
    tag = "workspaces",
    params(
        ("workspace_id" = String, Path, description = "Workspace id"),
        ("placement_id" = String, Path, description = "Placement id")
    ),
    responses(
        (status = 200, description = "Repo removed", body = WorkspaceResponse),
        (status = 403, description = "Forbidden", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn remove_workspace_repo(
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
    Path((workspace_id, placement_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    super::mutations::remove_workspace_repo(
        workspaces.store(),
        &auth.scope_context(),
        workspace_id.clone(),
        placement_id,
    )
    .await?;
    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/repos/{placement_id}/primary",
    tag = "workspaces",
    params(
        ("workspace_id" = String, Path, description = "Workspace id"),
        ("placement_id" = String, Path, description = "Placement id")
    ),
    responses(
        (status = 200, description = "Primary set", body = WorkspaceResponse),
        (status = 403, description = "Forbidden", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn set_workspace_primary_repo(
    State(workspaces): State<Arc<Workspaces>>,
    auth: AuthUser,
    Path((workspace_id, placement_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceResponse>, WorkspaceError> {
    super::mutations::set_workspace_primary_repo(
        workspaces.store(),
        &auth.scope_context(),
        workspace_id.clone(),
        placement_id,
    )
    .await?;
    let response = load_response(&workspaces, &auth, &workspace_id).await?;
    Ok(Json(response))
}
