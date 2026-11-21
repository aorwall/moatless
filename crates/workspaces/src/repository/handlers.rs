//! Repository HTTP handlers.

use super::error::RepositoryError;
use super::operations::Repositories;
use super::schemas::*;
use crate::workspace::git_sync::GitSyncService;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use moatless_core::db::DbPool;
use moatless_core::http_auth::AuthUser;
use std::sync::Arc;

/// Create a new repository.
#[utoipa::path(
    post,
    path = "/api/v1/repositories",
    tag = "repositories",
    request_body = CreateRepositoryRequest,
    responses(
        (status = 201, description = "Repository created", body = RepositoryResponse),
        (status = 409, description = "Repository already exists", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn create_repository(
    State(repositories): State<Arc<Repositories>>,
    State(git_sync): State<Arc<GitSyncService>>,
    auth: AuthUser,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryResponse>), RepositoryError> {
    let row = super::mutations::create_repository(
        repositories.store(),
        &git_sync,
        &auth.scope_context(),
        req,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(repository_response(row))))
}

/// List all repositories for the current user.
#[utoipa::path(
    get,
    path = "/api/v1/repositories",
    tag = "repositories",
    responses(
        (status = 200, description = "List of repositories", body = Vec<RepositoryResponse>)
    )
)]
pub async fn list_repositories(
    State(repositories): State<Arc<Repositories>>,
    auth: AuthUser,
) -> Result<Json<Vec<RepositoryResponse>>, RepositoryError> {
    let rows = repositories.store().list_all(&auth.scope_context()).await?;
    let repos: Vec<RepositoryResponse> = rows.into_iter().map(repository_response).collect();
    Ok(Json(repos))
}

/// Get a repository by ID.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{repository_id}",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "Repository details", body = RepositoryResponse),
        (status = 404, description = "Repository not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn get_repository(
    State(repositories): State<Arc<Repositories>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let row = repositories
        .store()
        .find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(repository_response(row)))
}

/// Update a repository by ID.
#[utoipa::path(
    put,
    path = "/api/v1/repositories/{repository_id}",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    request_body = UpdateRepositoryRequest,
    responses(
        (status = 200, description = "Repository updated", body = RepositoryResponse),
        (status = 404, description = "Repository not found", body = moatless_types::http::ErrorBody),
        (status = 409, description = "Repository URL already exists", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn update_repository(
    State(repositories): State<Arc<Repositories>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
    Json(req): Json<UpdateRepositoryRequest>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let row = super::mutations::update_repository(
        repositories.store(),
        &auth.scope_context(),
        repository_id,
        req,
    )
    .await?;
    Ok(Json(repository_response(row)))
}

/// Delete a repository by ID (soft delete).
#[utoipa::path(
    delete,
    path = "/api/v1/repositories/{repository_id}",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 204, description = "Repository deleted"),
        (status = 404, description = "Repository not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn delete_repository(
    State(repositories): State<Arc<Repositories>>,
    State(db): State<DbPool>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<StatusCode, RepositoryError> {
    super::mutations::delete_repository(
        repositories.store(),
        &db,
        &auth.scope_context(),
        repository_id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
