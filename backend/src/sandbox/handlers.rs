//! Sandbox handler functions.
//!
//! Extracted from routes.rs for consistency with other modules
//! (task/, message/, agent/, etc.).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;
use tracing::{error, info};
use utoipa::IntoParams;

use super::error::SandboxError;
use super::routes::SandboxApiState;
use super::models::*;
use super::schemas::*;

/// Provision a new sandbox workspace for a task.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/provision",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = ProvisionRequest,
    responses(
        (status = 200, description = "Workspace provisioned successfully", body = ProvisionResponse),
        (status = 500, description = "Provisioning failed", body = ErrorBody)
    )
)]
pub async fn provision_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
    Json(request): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, SandboxError> {
    info!(task_id = %task_id, "POST /provision");
    match state.provider.provision(&task_id, &request).await {
        Ok(response) => {
            info!(task_id = %task_id, "Provision succeeded");
            Ok(Json(response))
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Provision failed");
            Err(e)
        }
    }
}

/// Get environment status for a task workspace.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/env-status",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Environment status", body = StatusResponse),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn status_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.provider.status(&task_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            error!(task_id = %task_id, error = %e, "env-status failed");
            e.into_response()
        }
    }
}

/// Check if a task's sandbox is healthy.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/health",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Health check result", body = HealthResponse)
    )
)]
pub async fn health_check_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let resp = state.provider.health(&task_id).await;
    Json(resp)
}

/// Get full workspace status including servers and agent state.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/status",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Full workspace status", body = WorkspaceStatusFullResponse),
        (status = 503, description = "Workspace not ready", body = ErrorBody),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn sandbox_status_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.provider.sandbox_status(&task_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            error!(task_id = %task_id, error = %e, "sandbox status failed");
            e.into_response()
        }
    }
}

/// Stop a running workspace.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/stop",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Workspace stopped"),
        (status = 500, description = "Stop failed", body = ErrorBody)
    )
)]
pub async fn stop_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /stop");
    match state.provider.stop(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Stop succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Stop failed");
            Err(e)
        }
    }
}

/// Start a stopped workspace.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/start",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Workspace started"),
        (status = 500, description = "Start failed", body = ErrorBody)
    )
)]
pub async fn start_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /start");
    match state.provider.start(&task_id).await {
        Ok(_result) => {
            info!(task_id = %task_id, "Start succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Start failed");
            Err(e)
        }
    }
}

/// Restart a workspace (stop + start).
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/restart",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Workspace restarted"),
        (status = 500, description = "Restart failed", body = ErrorBody)
    )
)]
pub async fn restart_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /restart");
    match state.provider.restart(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Restart succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Restart failed");
            Err(e)
        }
    }
}

/// Clean up a workspace and all associated resources.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/cleanup",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 204, description = "Workspace cleaned up"),
        (status = 500, description = "Cleanup failed", body = ErrorBody)
    )
)]
pub async fn cleanup_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, SandboxError> {
    info!(task_id = %task_id, "POST /cleanup");
    match state.provider.cleanup(&task_id).await {
        Ok(()) => {
            info!(task_id = %task_id, "Cleanup succeeded");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            error!(task_id = %task_id, error = %e, "Cleanup failed");
            Err(e)
        }
    }
}

// =============================================================================
// New preview/workspace handlers
// =============================================================================

/// Get server status for all servers in a task's sandbox.
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/server/status",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Server status", body = ServerStatusResponse),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn server_status_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.provider.server_status(&task_id).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            error!(task_id = %task_id, error = %e, "server status failed");
            e.into_response()
        }
    }
}

/// Stream server logs via SSE.
///
/// Returns a Server-Sent Events stream of log lines for the given server.
/// The response body cannot be represented in OpenAPI (SSE streaming).
#[utoipa::path(
    get,
    path = "/api/sandbox/v1/tasks/{task_id}/servers/{server_name}/logs",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("server_name" = String, Path, description = "Server name")
    ),
    responses(
        (status = 200, description = "SSE stream of server log lines"),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn server_logs_handler(
    State(state): State<SandboxApiState>,
    Path((task_id, server_name)): Path<(String, String)>,
) -> Result<Sse<impl Stream<Item = Result<Event, SandboxError>>>, SandboxError> {
    let stream = state
        .provider
        .server_logs(&task_id, &server_name)
        .await?;
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Search files in a task's sandbox workspace.
#[utoipa::path(
    post,
    path = "/api/sandbox/v1/tasks/{task_id}/search-files",
    tag = "sandbox",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body = SearchFilesRequest,
    responses(
        (status = 200, description = "File search results", body = Vec<FileSearchResult>),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn search_files_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
    Json(req): Json<SearchFilesRequest>,
) -> Result<Json<Vec<FileSearchResult>>, SandboxError> {
    let results = state
        .provider
        .search_files(&task_id, &req.query, req.limit.unwrap_or(20))
        .await?;
    Ok(Json(results))
}

// =============================================================================
// File operation handlers
// =============================================================================

/// Query parameters for the file tree endpoint.
#[derive(Debug, Deserialize, IntoParams)]
pub struct FileTreeQuery {
    /// Root path relative to workspace (default ".")
    #[serde(default)]
    pub path: Option<String>,
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
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn file_tree_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
    Query(query): Query<FileTreeQuery>,
) -> Result<Json<FileTreeResponse>, SandboxError> {
    let root_path = query.path.as_deref().unwrap_or(".");
    let result = state.provider.get_file_tree(&task_id, root_path).await?;
    Ok(Json(result))
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
        (status = 404, description = "File not found", body = ErrorBody),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn read_file_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
    Query(query): Query<ReadFileQuery>,
) -> Result<Json<FileContentResponse>, SandboxError> {
    let include_changes = query.changes.unwrap_or(false);
    let result = state
        .provider
        .read_file(&task_id, &query.path, include_changes)
        .await?;
    Ok(Json(result))
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
        (status = 400, description = "Invalid path", body = ErrorBody),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn write_file_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
    Json(req): Json<WriteFileRequestBody>,
) -> Result<StatusCode, SandboxError> {
    state
        .provider
        .write_file(&task_id, &req.path, &req.content)
        .await?;
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
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn list_files_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
    Query(query): Query<ListFilesQuery>,
) -> Result<Json<ListFilesResponse>, SandboxError> {
    let path = query.path.as_deref().unwrap_or(".");
    let recursive = query.recursive.unwrap_or(false);
    let result = state.provider.list_files(&task_id, path, recursive).await?;
    Ok(Json(result))
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
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub async fn search_by_name_handler(
    State(state): State<SandboxApiState>,
    Path(task_id): Path<String>,
    Json(req): Json<SearchByNameRequest>,
) -> Result<Json<SearchByNameResponse>, SandboxError> {
    let result = state
        .provider
        .search_files_by_name(
            &task_id,
            &req.query,
            req.case_sensitive.unwrap_or(false),
            req.max_results.unwrap_or(0),
        )
        .await?;
    Ok(Json(result))
}
