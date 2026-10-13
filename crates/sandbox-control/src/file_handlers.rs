//! File operation handlers for sandbox workspaces.
//!
//! These handlers receive a task-addressed client from `SandboxConnector`, the
//! only code that knows runtime-specific gRPC addressing.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::IntoParams;

use super::access::{authorize_task_control, authorize_task_read};
use super::routes::SandboxApiState;
use base64::Engine;
use moatless_core::http_auth::OptionalScope;
use moatless_ports::sandbox::{
    self as core_sandbox, SnapshotError, SnapshotListing, WorkspaceSnapshotSource,
};
use moatless_sandbox_runtime::client::classify_grpc_error;
use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::schemas::*;
use moatless_sandbox_runtime::workspace_copy::{self, WorkspaceCopy};

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
    /// Include git change info (`old_content`, additions, deletions)
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

/// See `sandbox::files::tree::get_file_tree_with_changes` for the file-tree
/// contract.
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
        (status = 503, description = "Sandbox not ready", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
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
    let client = match open_copy(&state, &task_id).await? {
        WorkspaceCopy::Live(client) => client,
        WorkspaceCopy::Stored {
            source,
            unreachable,
        } => return snapshot_tree(source, &task_id, root_path, recursive, unreachable).await,
    };
    let resp = client
        .get_file_tree_for_task(Some(&task_id), root_path, recursive)
        .await
        .map_err(|e| classify_grpc_error(e, "get_file_tree"))?;
    Ok(Json(moatless_sandbox_runtime::file_conversion::file_tree(
        resp,
    )?))
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
        (status = 400, description = "Invalid path or path is a directory", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "File not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Sandbox not ready", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
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
    let client = match open_copy(&state, &task_id).await? {
        WorkspaceCopy::Live(client) => client,
        WorkspaceCopy::Stored {
            source,
            unreachable,
        } => return snapshot_read(source, &task_id, &query.path, unreachable).await,
    };
    let resp = client
        .read_file_for_task(Some(&task_id), &query.path, include_changes)
        .await
        .map_err(|e| classify_grpc_error(e, "read_file"))?;
    Ok(Json(
        moatless_sandbox_runtime::file_conversion::file_content(resp),
    ))
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
        (status = 400, description = "Invalid path", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Sandbox not ready", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
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
        (status = 503, description = "Sandbox not ready", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
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
    let client = match open_copy(&state, &task_id).await? {
        WorkspaceCopy::Live(client) => client,
        WorkspaceCopy::Stored {
            source,
            unreachable,
        } => return snapshot_list(source, &task_id, path, recursive, unreachable).await,
    };
    let files = client
        // The whole walk, which is what this endpoint has always returned.
        .list_files_for_task(Some(&task_id), path, recursive, false)
        .await
        .map_err(|e| classify_grpc_error(e, "list_files"))?;
    Ok(Json(ListFilesResponse {
        files,
        origin: ContentOrigin::sandbox(),
    }))
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
        (status = 503, description = "Sandbox not ready", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn search_by_name_handler(
    State(state): State<SandboxApiState>,
    OptionalScope(scope): OptionalScope,
    Path(task_id): Path<String>,
    Json(req): Json<SearchByNameRequest>,
) -> Result<Json<SearchByNameResponse>, SandboxError> {
    authorize_task_read(&state, &scope, &task_id).await?;
    let client = match open_copy(&state, &task_id).await? {
        WorkspaceCopy::Live(client) => client,
        WorkspaceCopy::Stored {
            source,
            unreachable,
        } => return snapshot_search(source, &task_id, &req, unreachable).await,
    };
    let resp = client
        .search_files_by_name_for_task(
            Some(&task_id),
            &req.query,
            req.case_sensitive.unwrap_or(false),
            req.max_results.unwrap_or(0),
        )
        .await
        .map_err(|e| classify_grpc_error(e, "search_files_by_name"))?;
    Ok(Json(
        moatless_sandbox_runtime::file_conversion::search_by_name(resp),
    ))
}

// ── Snapshot fallback ────────────────────────────────────────────
//
// When no sandbox is running, a task's files are still readable from the
// S3 snapshot (`workspace_snapshot`). Each read handler falls through to
// its counterpart here when `workspace_copy::open` picks the stored copy.

/// [`workspace_copy::open`] for this state; both error cases answer with the
/// connector's error.
async fn open_copy<'a>(
    state: &'a SandboxApiState,
    task_id: &str,
) -> Result<WorkspaceCopy<'a>, SandboxError> {
    workspace_copy::open(&*state.connector, state.snapshots.as_deref(), task_id)
        .await
        .map_err(|e| e.into_sandbox_error().into())
}

/// Map a snapshot miss onto the response error.
///
/// `NotFound` is the path's own 404; `NoSnapshot` and `Unavailable` read back
/// as the sandbox failure that sent the read here — nothing running and nothing
/// stored means not running, and an outage is not this path's fault.
fn snapshot_failed(
    task_id: &str,
    what: &str,
    error: SnapshotError,
    unreachable: core_sandbox::SandboxError,
) -> SandboxError {
    match error {
        SnapshotError::NotFound => SandboxError::NotFound(what.to_string()),
        SnapshotError::NoSnapshot => unreachable.into(),
        SnapshotError::Unavailable(message) => {
            tracing::warn!(task_id = %task_id, %message, "Could not read the stored workspace");
            unreachable.into()
        }
    }
}

async fn snapshot_read(
    source: &dyn WorkspaceSnapshotSource,
    task_id: &str,
    path: &str,
    unreachable: core_sandbox::SandboxError,
) -> Result<Json<FileContentResponse>, SandboxError> {
    let file = source
        .read(task_id, path)
        .await
        .map_err(|e| snapshot_failed(task_id, path, e, unreachable))?;

    // UTF-8 or base64, the same distinction the live read draws — the client
    // renders a binary file from the encoding and `mediaType`.
    let (content, is_binary) = match String::from_utf8(file.content) {
        Ok(text) => (text, false),
        Err(err) => (
            base64::engine::general_purpose::STANDARD.encode(err.as_bytes()),
            true,
        ),
    };
    Ok(Json(FileContentResponse {
        content,
        // No git overlay: the snapshot is not a working tree with a HEAD to
        // diff against.
        old_content: None,
        additions: None,
        deletions: None,
        is_modified: None,
        is_binary: Some(is_binary),
        media_type: media_type(path),
        origin: ContentOrigin::snapshot(file.synced_at),
    }))
}

async fn snapshot_tree(
    source: &dyn WorkspaceSnapshotSource,
    task_id: &str,
    root: &str,
    recursive: bool,
    unreachable: core_sandbox::SandboxError,
) -> Result<Json<FileTreeResponse>, SandboxError> {
    let listing = source
        .list(task_id, root, recursive)
        .await
        .map_err(|e| snapshot_failed(task_id, root, e, unreachable))?;
    let synced_at = listing.synced_at;
    Ok(Json(FileTreeResponse {
        tree: tree_nodes(root, &listing, recursive),
        git_summary: GitSummaryDto {
            total_files: 0,
            total_additions: 0,
            total_deletions: 0,
            total_staged: 0,
            total_modified: 0,
            compare_base: "HEAD".to_string(),
            has_changes: false,
        },
        origin: ContentOrigin::snapshot(synced_at),
    }))
}

async fn snapshot_list(
    source: &dyn WorkspaceSnapshotSource,
    task_id: &str,
    path: &str,
    recursive: bool,
    unreachable: core_sandbox::SandboxError,
) -> Result<Json<ListFilesResponse>, SandboxError> {
    let listing = source
        .list(task_id, path, recursive)
        .await
        .map_err(|e| snapshot_failed(task_id, path, e, unreachable))?;
    let synced_at = listing.synced_at;
    // This endpoint lists files, not directories, matching the live walk.
    let files = listing
        .entries
        .into_iter()
        .filter(|e| !e.is_dir)
        .map(|e| e.path)
        .collect();
    Ok(Json(ListFilesResponse {
        files,
        origin: ContentOrigin::snapshot(synced_at),
    }))
}

async fn snapshot_search(
    source: &dyn WorkspaceSnapshotSource,
    task_id: &str,
    req: &SearchByNameRequest,
    unreachable: core_sandbox::SandboxError,
) -> Result<Json<SearchByNameResponse>, SandboxError> {
    let limit = req.max_results.unwrap_or(0).max(0) as usize;
    let listing = source
        .search(
            task_id,
            &req.query,
            req.case_sensitive.unwrap_or(false),
            limit,
        )
        .await
        .map_err(|e| snapshot_failed(task_id, &req.query, e, unreachable))?;
    let synced_at = listing.synced_at;
    let files: Vec<FileNameMatchDto> = listing
        .entries
        .into_iter()
        .filter(|e| !e.is_dir)
        .map(|e| FileNameMatchDto {
            path: e.path,
            // The snapshot search matches on the path, not on content, so it
            // has no per-file hit count to report.
            matches: 0,
        })
        .collect();
    Ok(Json(SearchByNameResponse {
        total_matches: files.len() as i32,
        files,
        origin: ContentOrigin::snapshot(synced_at),
    }))
}

/// The nested file tree for one snapshot listing.
///
/// A directory always carries a child list (possibly empty), a file never does
/// — matching [`FileTreeNodeDto`]'s live contract of telling "expandable" from
/// "not" via `Some`/`None`; a non-recursive listing gives the root's direct
/// children only, so its directories come back expandable but not yet expanded.
fn tree_nodes(root: &str, listing: &SnapshotListing, recursive: bool) -> Vec<FileTreeNodeDto> {
    use std::collections::BTreeMap;
    let mut children_of: BTreeMap<&str, Vec<&core_sandbox::SnapshotEntry>> = BTreeMap::new();
    for entry in &listing.entries {
        let parent = entry.path.rsplit_once('/').map_or("", |(p, _)| p);
        children_of.entry(parent).or_default().push(entry);
    }
    let root_key = root.trim_matches('/');
    let root_key = if root_key == "." { "" } else { root_key };
    children_of
        .get(root_key)
        .map(|kids| {
            kids.iter()
                .map(|e| tree_node(e, &children_of, recursive))
                .collect()
        })
        .unwrap_or_default()
}

fn tree_node(
    entry: &core_sandbox::SnapshotEntry,
    children_of: &std::collections::BTreeMap<&str, Vec<&core_sandbox::SnapshotEntry>>,
    recursive: bool,
) -> FileTreeNodeDto {
    let name = entry
        .path
        .rsplit_once('/')
        .map_or(entry.path.as_str(), |(_, n)| n)
        .to_string();
    let children = entry.is_dir.then(|| {
        if recursive {
            children_of
                .get(entry.path.as_str())
                .map(|kids| {
                    kids.iter()
                        .map(|e| tree_node(e, children_of, recursive))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    });
    FileTreeNodeDto {
        name,
        path: entry.path.clone(),
        node_type: if entry.is_dir { "directory" } else { "file" }.to_string(),
        is_changed: false,
        git_status: None,
        additions: None,
        deletions: None,
        is_staged: None,
        children,
    }
}

/// The media type a path's extension names, for the image and PDF assets a
/// stopped Task can still be asked to serve. `None` leaves it to the caller,
/// which is what the live read does for anything it decoded as text.
fn media_type(path: &str) -> Option<String> {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => return None,
    };
    Some(mime.to_string())
}
