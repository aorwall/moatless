//! Memory HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use crate::task::operations::TaskCommands;
use moatless_core::http_auth::AuthUser;

use super::error::MemoryError;
use super::operations::Memories;
use super::schemas::{
    AppendMemoryRequest, ClaimMemoryRequest, GetMemoryResponse, ListMemoryResponse,
    MemoryEntryResponse, MemoryKeyRequest, MemoryLogResponse, MemoryScope, PutMemoryRequest,
};
use super::service::{self, ListParams};

/// Query naming one entry, or a `prefix*`.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EntryQuery {
    pub key: String,
    /// Every scope the Task reaches, narrowest first, when omitted.
    pub scope: Option<MemoryScope>,
}

/// Query of the list route.
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    pub scope: Option<MemoryScope>,
    pub prefix: Option<String>,
    pub tag: Option<String>,
    pub updated_since_seconds: Option<i64>,
    pub limit: Option<i64>,
}

/// Query of the search route.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// A web-search expression: words, `"phrases"`, `-excluded`, `or`.
    pub q: String,
    pub scope: Option<MemoryScope>,
    pub limit: Option<i64>,
}

/// Query of the log route.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LogQuery {
    pub key: String,
    pub scope: Option<MemoryScope>,
    pub limit: Option<i64>,
}

/// List summaries of the memory entries a Task reaches, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/memory",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier"), ListQuery),
    responses(
        (status = 200, description = "Entry summaries", body = ListMemoryResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<ListMemoryResponse>, MemoryError> {
    let params = ListParams {
        scope: query.scope,
        prefix: query.prefix,
        tag: query.tag,
        updated_since_seconds: query.updated_since_seconds,
        limit: query.limit,
    };
    let items = service::list(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        params,
    )
    .await?;
    Ok(Json(ListMemoryResponse { items }))
}

/// Read whole entries and record the read.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/memory/entry",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier"), EntryQuery),
    responses(
        (status = 200, description = "The entry, or every match of a prefix", body = GetMemoryResponse),
        (status = 404, description = "Task or entry not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<EntryQuery>,
) -> Result<Json<GetMemoryResponse>, MemoryError> {
    let items = service::get(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        &query.key,
        query.scope,
    )
    .await?;
    Ok(Json(GetMemoryResponse { items }))
}

/// Create or update an entry.
#[utoipa::path(
    put,
    path = "/api/v1/tasks/{task_id}/memory/entry",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier")),
    request_body = PutMemoryRequest,
    responses(
        (status = 200, description = "The entry as written", body = MemoryEntryResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Caller cannot write this memory", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "ifVersion did not match", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn put_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<PutMemoryRequest>,
) -> Result<Json<MemoryEntryResponse>, MemoryError> {
    let entry = service::put(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        req,
    )
    .await?;
    Ok(Json(entry))
}

/// Delete an entry and its history.
#[utoipa::path(
    delete,
    path = "/api/v1/tasks/{task_id}/memory/entry",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier"), EntryQuery),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "Caller cannot write this memory", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task or entry not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<EntryQuery>,
) -> Result<StatusCode, MemoryError> {
    service::delete(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        &query.key,
        query.scope,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Search entry summaries, best match first.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/memory/search",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier"), SearchQuery),
    responses(
        (status = 200, description = "Matching entry summaries", body = ListMemoryResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn search_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<ListMemoryResponse>, MemoryError> {
    let items = service::search(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        &query.q,
        query.scope,
        query.limit,
    )
    .await?;
    Ok(Json(ListMemoryResponse { items }))
}

/// Read an entry's history, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/memory/log",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier"), LogQuery),
    responses(
        (status = 200, description = "The entry's history", body = MemoryLogResponse),
        (status = 404, description = "Task or entry not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn memory_log(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<LogQuery>,
) -> Result<Json<MemoryLogResponse>, MemoryError> {
    let log = service::log(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        &query.key,
        query.scope,
        query.limit,
    )
    .await?;
    Ok(Json(log))
}

/// Add a dated line to an entry's history.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/memory/append",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier")),
    request_body = AppendMemoryRequest,
    responses(
        (status = 200, description = "The entry", body = MemoryEntryResponse),
        (status = 404, description = "Task or entry not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn append_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<AppendMemoryRequest>,
) -> Result<Json<MemoryEntryResponse>, MemoryError> {
    let entry = service::append(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        req,
    )
    .await?;
    Ok(Json(entry))
}

/// Record that an entry's citations still hold, renewing its expiry.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/memory/verify",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier")),
    request_body = MemoryKeyRequest,
    responses(
        (status = 200, description = "The entry", body = MemoryEntryResponse),
        (status = 404, description = "Task or entry not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn verify_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<MemoryKeyRequest>,
) -> Result<Json<MemoryEntryResponse>, MemoryError> {
    let entry = service::verify(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        &req.key,
        req.scope,
    )
    .await?;
    Ok(Json(entry))
}

/// Claim an entry for the Task.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/memory/claim",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier")),
    request_body = ClaimMemoryRequest,
    responses(
        (status = 200, description = "Claimed", body = MemoryEntryResponse),
        (status = 404, description = "Task or entry not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "Another open Task holds the claim", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn claim_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<ClaimMemoryRequest>,
) -> Result<Json<MemoryEntryResponse>, MemoryError> {
    let entry = service::claim(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        req,
    )
    .await?;
    Ok(Json(entry))
}

/// Drop the Task's claim on an entry.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/memory/release",
    tag = "memory",
    params(("task_id" = String, Path, description = "Task identifier")),
    request_body = MemoryKeyRequest,
    responses(
        (status = 204, description = "Released"),
        (status = 404, description = "Task or entry not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "The Task holds no claim", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn release_memory(
    State(tasks): State<Arc<TaskCommands>>,
    State(memories): State<Arc<Memories>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<MemoryKeyRequest>,
) -> Result<StatusCode, MemoryError> {
    service::release(
        tasks.as_ref(),
        memories.as_ref(),
        &auth.scope_context(),
        &task_id,
        &req.key,
        req.scope,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
