//! File operation handlers for sandbox workspaces.
//!
//! These handlers receive a task-addressed client from `SandboxConnector`, the
//! only code that knows runtime-specific gRPC addressing.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::IntoParams;

use super::client::classify_grpc_error;
use super::error::SandboxError;
use super::handlers::{authorize_task_control, authorize_task_read};
use super::routes::SandboxApiState;
use super::schemas::*;
use moatless_identity::extractors::OptionalScope;

/// Query parameters for the file tree endpoint.
#[derive(Debug, Deserialize, IntoParams)]
pub struct FileTreeQuery {
    /// Root path relative to workspace (default ".")
    #[serde(default)]
    pub path: Option<String>,
    /// Walk subdirectories (default false: only direct children of `path`)
    #[serde(default)]
    pub recursive: Option<bool>,
}

/// Query parameters for reading a file.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ReadFileQuery {
    /// File path relative to workspace
    pub path: String,
    /// Include git change info (old_content, additions, deletions)
    #[serde(default)]
    pub changes: Option<bool>,
}

/// Query parameters for listing files.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListFilesQuery {
    /// Path relative to workspace (default ".")
    #[serde(default)]
    pub path: Option<String>,
    /// List files recursively
    #[serde(default)]
    pub recursive: Option<bool>,
}

/// Get the file tree with git change overlay.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/files/tree",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        FileTreeQuery,
    ),
    responses(
        (status = 200, description = "File tree with git summary", body = FileTreeResponse),
        (status = 503, description = "Sandbox not ready", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn file_tree_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
    Query(query): Query<FileTreeQuery>,
) -> Result<Json<FileTreeResponse>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let root_path = query.path.as_deref().unwrap_or(".");
    let recursive = query.recursive.unwrap_or(false);
    let client = state.connector.sandbox_client(&task_id).await?;
    let resp = client
        .get_file_tree_for_task(Some(&task_id), root_path, recursive)
        .await
        .map_err(|e| classify_grpc_error(e, "get_file_tree"))?;
    Ok(Json(FileTreeResponse::try_from(resp)?))
}

/// Read a file from the sandbox workspace.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/files/read",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ReadFileQuery,
    ),
    responses(
        (status = 200, description = "File content", body = FileContentResponse),
        (status = 400, description = "Invalid path or path is a directory", body = moatless_core::error::ErrorBody),
        (status = 404, description = "File not found", body = moatless_core::error::ErrorBody),
        (status = 503, description = "Sandbox not ready", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn read_file_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
    Query(query): Query<ReadFileQuery>,
) -> Result<Json<FileContentResponse>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let include_changes = query.changes.unwrap_or(false);
    let client = state.connector.sandbox_client(&task_id).await?;
    let resp = client
        .read_file_for_task(Some(&task_id), &query.path, include_changes)
        .await
        .map_err(|e| classify_grpc_error(e, "read_file"))?;
    Ok(Json(FileContentResponse::from(resp)))
}

/// Write a file in the sandbox workspace.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/files/write",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = WriteFileRequestBody,
    responses(
        (status = 204, description = "File written successfully"),
        (status = 400, description = "Invalid path", body = moatless_core::error::ErrorBody),
        (status = 503, description = "Sandbox not ready", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn write_file_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
    Json(req): Json<WriteFileRequestBody>,
) -> Result<StatusCode, SandboxError> {
    authorize_task_control(&state, &scope, &task_id).await?;
    let client = state.connector.sandbox_client(&task_id).await?;
    client
        .write_file_for_task(Some(&task_id), &req.path, &req.content)
        .await
        .map_err(|e| classify_grpc_error(e, "write_file"))?;
    Ok(StatusCode::NO_CONTENT)
}

/// List files in the sandbox workspace.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/files/list",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ListFilesQuery,
    ),
    responses(
        (status = 200, description = "File list", body = ListFilesResponse),
        (status = 503, description = "Sandbox not ready", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn list_files_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
    Query(query): Query<ListFilesQuery>,
) -> Result<Json<ListFilesResponse>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let path = query.path.as_deref().unwrap_or(".");
    let recursive = query.recursive.unwrap_or(false);
    let client = state.connector.sandbox_client(&task_id).await?;
    let files = client
        // The whole walk, which is what this endpoint has always returned.
        .list_files_for_task(Some(&task_id), path, recursive, false)
        .await
        .map_err(|e| classify_grpc_error(e, "list_files"))?;
    Ok(Json(ListFilesResponse { files }))
}

/// Search files by name in the sandbox workspace.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/files/search-by-name",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = SearchByNameRequest,
    responses(
        (status = 200, description = "Search results", body = SearchByNameResponse),
        (status = 503, description = "Sandbox not ready", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn search_by_name_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
    Json(req): Json<SearchByNameRequest>,
) -> Result<Json<SearchByNameResponse>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let client = state.connector.sandbox_client(&task_id).await?;
    let resp = client
        .search_files_by_name_for_task(
            Some(&task_id),
            &req.query,
            req.case_sensitive.unwrap_or(false),
            req.max_results.unwrap_or(0),
        )
        .await
        .map_err(|e| classify_grpc_error(e, "search_files_by_name"))?;
    Ok(Json(SearchByNameResponse::from(resp)))
}
