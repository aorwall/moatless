//! Artifact record HTTP handlers: the page routes a served page calls with its
//! page token, and the user routes the CLI calls.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::message::send::MessageSendService;
use crate::task::operations::TaskCommands;
use moatless_api_schemas::collection::ListCollectionRecordsResponse;
use moatless_core::http_auth::AuthUser;

use super::error::ArtifactError;
use super::operations::Artifacts;
use super::schemas::{
    ArtifactRecordResponse, ListArtifactRecordsResponse, WriteArtifactRecordRequest,
};

/// Query parameters of the record lists.
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRecordsQuery {
    /// RFC 3339 time; only records changed after it are listed.
    pub since: Option<String>,
    /// Only records whose keys start with it, such as `comments/` for a list.
    pub prefix: Option<String>,
}

impl ListRecordsQuery {
    fn filter(&self) -> super::records::RecordFilter<'_> {
        super::records::RecordFilter {
            prefix: self.prefix.as_deref(),
            since: self.since.as_deref(),
        }
    }
}

/// Save, append or delete a record from a served page, queuing a message to
/// the Task's agent when the Artifact was published with `notify`.
#[utoipa::path(
    post,
    path = "/api/artifact-data/{token}/records",
    tag = "artifacts",
    params(("token" = String, Path, description = "Page token the served page carries")),
    request_body(content = WriteArtifactRecordRequest, description = "JSON, sent as any content type"),
    responses(
        (status = 200, description = "The record written, or as it was before a delete", body = ArtifactRecordResponse),
        (status = 400, description = "Invalid record", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Invalid or expired page token", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "The latest publish accepts no data", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No record under the key to delete", body = moatless_api_schemas::error::ErrorBody),
        (status = 413, description = "Record too large", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn write_page_record(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    State(messages): State<Arc<MessageSendService>>,
    Path(token): Path<String>,
    body: Bytes,
) -> Response {
    let saved = super::records::write_from_page(
        tasks.as_ref(),
        artifacts.as_ref(),
        messages.as_ref(),
        &token,
        &body,
    )
    .await;
    page_response(saved.map(Json))
}

/// List the records of the Artifact a page token names, oldest change first.
#[utoipa::path(
    get,
    path = "/api/artifact-data/{token}/records",
    tag = "artifacts",
    params(
        ("token" = String, Path, description = "Page token the served page carries"),
        ListRecordsQuery
    ),
    responses(
        (status = 200, description = "Record list", body = ListArtifactRecordsResponse),
        (status = 401, description = "Invalid or expired page token", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_page_records(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    Path(token): Path<String>,
    Query(query): Query<ListRecordsQuery>,
) -> Response {
    let items =
        super::records::list_for_page(tasks.as_ref(), artifacts.as_ref(), &token, query.filter())
            .await;
    page_response(items.map(|items| Json(ListArtifactRecordsResponse { items })))
}

/// List the records of a scoped collection attached to the Artifact a page
/// token names, oldest change first. A page reads it and never writes it.
#[utoipa::path(
    get,
    path = "/api/artifact-data/{token}/collections/{name}/records",
    tag = "artifacts",
    params(
        ("token" = String, Path, description = "Page token the served page carries"),
        ("name" = String, Path, description = "Collection name the Artifact was published with"),
        ListRecordsQuery
    ),
    responses(
        (status = 200, description = "Record list", body = ListCollectionRecordsResponse),
        (status = 401, description = "Invalid or expired page token", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "The reader cannot read the collection's scope", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No collection by that name on the Artifact", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_page_collection_records(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    Path((token, name)): Path<(String, String)>,
    Query(query): Query<ListRecordsQuery>,
) -> Response {
    let items = super::records::list_collection_for_page(
        tasks.as_ref(),
        artifacts.as_ref(),
        &token,
        &name,
        query.filter(),
    )
    .await;
    page_response(items.map(|items| Json(ListCollectionRecordsResponse { items })))
}

/// Answer a preflight for a page that calls the routes with its own
/// `fetch` and a JSON content type, rather than through the page script.
pub async fn page_records_preflight() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    response
}

/// `result` readable from a served page, error included. A served page has an
/// opaque origin, and the token in the path is the only credential, so any
/// origin may read the answer.
fn page_response(result: Result<impl IntoResponse, ArtifactError>) -> Response {
    let mut response = match result {
        Ok(ok) => ok.into_response(),
        Err(e) => e.into_response(),
    };
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

/// Save, append or delete a record as the caller, who must be able to change
/// the Task. Queues no message.
#[utoipa::path(
    post,
    path = "/api/v1/artifacts/{artifact_id}/records",
    tag = "artifacts",
    params(("artifact_id" = String, Path, description = "Artifact identifier")),
    request_body = WriteArtifactRecordRequest,
    responses(
        (status = 200, description = "The record written, or as it was before a delete", body = ArtifactRecordResponse),
        (status = 400, description = "Invalid record", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Caller cannot change the Task", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Artifact, or the record to delete, not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn write_record(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(artifact_id): Path<String>,
    Json(req): Json<WriteArtifactRecordRequest>,
) -> Result<Json<ArtifactRecordResponse>, ArtifactError> {
    let record = super::records::write_as_user(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
        req,
    )
    .await?;
    Ok(Json(record))
}

/// List an Artifact's records, oldest change first.
#[utoipa::path(
    get,
    path = "/api/v1/artifacts/{artifact_id}/records",
    tag = "artifacts",
    params(
        ("artifact_id" = String, Path, description = "Artifact identifier"),
        ListRecordsQuery
    ),
    responses(
        (status = 200, description = "Record list", body = ListArtifactRecordsResponse),
        (status = 404, description = "Artifact not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_records(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(artifact_id): Path<String>,
    Query(query): Query<ListRecordsQuery>,
) -> Result<Json<ListArtifactRecordsResponse>, ArtifactError> {
    let items = super::records::list(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
        query.filter(),
    )
    .await?;
    Ok(Json(ListArtifactRecordsResponse { items }))
}
