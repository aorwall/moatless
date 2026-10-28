//! Record collection HTTP handlers, for the agent's CLI.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::task::operations::TaskCommands;
use moatless_api_schemas::artifact::WriteArtifactRecordRequest;
use moatless_api_schemas::collection::{
    CollectionRecordResponse, ListCollectionRecordsResponse, ListCollectionsResponse, MemoryScope,
};
use moatless_core::http_auth::AuthUser;

use super::error::CollectionError;
use super::operations::Records;
use super::service::{self, RecordFilter};

/// Query naming the scope of a collection route.
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ScopeQuery {
    /// The Task's Loop, else the Task, when omitted.
    pub scope: Option<MemoryScope>,
}

/// Query of the record list.
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRecordsQuery {
    /// The Task's Loop, else the Task, when omitted.
    pub scope: Option<MemoryScope>,
    /// RFC 3339 time; only records changed after it are listed.
    pub since: Option<String>,
    /// Only records whose keys start with it, such as `runs/` for a list.
    pub prefix: Option<String>,
}

/// List the collections in the scope a Task reaches, by name.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/collections",
    tag = "collections",
    params(("task_id" = String, Path, description = "Task identifier"), ScopeQuery),
    responses(
        (status = 200, description = "Collections", body = ListCollectionsResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_collections(
    State(tasks): State<Arc<TaskCommands>>,
    State(records): State<Arc<Records>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<ListCollectionsResponse>, CollectionError> {
    let items = service::list_collections(
        tasks.as_ref(),
        records.as_ref(),
        &auth.scope_context(),
        &task_id,
        query.scope,
    )
    .await?;
    Ok(Json(ListCollectionsResponse { items }))
}

/// List a collection's records, oldest change first.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/collections/{name}/records",
    tag = "collections",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("name" = String, Path, description = "Collection name"),
        ListRecordsQuery
    ),
    responses(
        (status = 200, description = "Record list", body = ListCollectionRecordsResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_collection_records(
    State(tasks): State<Arc<TaskCommands>>,
    State(records): State<Arc<Records>>,
    auth: AuthUser,
    Path((task_id, name)): Path<(String, String)>,
    Query(query): Query<ListRecordsQuery>,
) -> Result<Json<ListCollectionRecordsResponse>, CollectionError> {
    let filter = RecordFilter {
        prefix: query.prefix.as_deref(),
        since: query.since.as_deref(),
    };
    let items = service::list(
        tasks.as_ref(),
        records.as_ref(),
        &auth.scope_context(),
        &task_id,
        &name,
        query.scope,
        filter,
    )
    .await?;
    Ok(Json(ListCollectionRecordsResponse { items }))
}

/// Save, append or delete a record, creating the collection on its first
/// write.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/collections/{name}/records",
    tag = "collections",
    params(
        ("task_id" = String, Path, description = "Task identifier"),
        ("name" = String, Path, description = "Collection name"),
        ScopeQuery
    ),
    request_body = WriteArtifactRecordRequest,
    responses(
        (status = 200, description = "The record written, or as it was before a delete", body = CollectionRecordResponse),
        (status = 400, description = "Invalid record", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Caller cannot write this scope's records", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task, or the record to delete, not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 413, description = "Record too large", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn write_collection_record(
    State(tasks): State<Arc<TaskCommands>>,
    State(records): State<Arc<Records>>,
    auth: AuthUser,
    Path((task_id, name)): Path<(String, String)>,
    Query(query): Query<ScopeQuery>,
    Json(req): Json<WriteArtifactRecordRequest>,
) -> Result<Json<CollectionRecordResponse>, CollectionError> {
    let record = service::write(
        tasks.as_ref(),
        records.as_ref(),
        &auth.scope_context(),
        &task_id,
        &name,
        query.scope,
        req,
    )
    .await?;
    Ok(Json(record))
}
