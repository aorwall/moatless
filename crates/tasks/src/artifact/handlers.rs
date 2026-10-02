//! Artifact HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Multipart;
use axum_extra::extract::multipart::MultipartError;

use crate::task::operations::TaskCommands;
use moatless_core::http_auth::AuthUser;

use super::error::ArtifactError;
use super::models::PublishRequest;
use super::operations::{Artifacts, PAGE_CONTENT_TYPE};
use super::schemas::{ArtifactResponse, ListArtifactsResponse, PublishArtifactBody};
use super::service::MAX_PAGE_BYTES;

/// Replaces axum's 2 MB default on the publish route: a full page plus room
/// for the multipart framing and text fields around it.
pub(crate) const PUBLISH_BODY_LIMIT: DefaultBodyLimit =
    DefaultBodyLimit::max(MAX_PAGE_BYTES + 1024 * 1024);

/// Publish a new version of an Artifact from a multipart upload.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/artifacts",
    tag = "artifacts",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body(content_type = "multipart/form-data", content = PublishArtifactBody),
    responses(
        (status = 200, description = "Version published", body = ArtifactResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Caller cannot change the Task", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 413, description = "Page too large", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Object storage not configured", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn publish_artifact(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    multipart: Multipart,
) -> Result<Json<ArtifactResponse>, ArtifactError> {
    if !artifacts.is_configured() {
        return Err(ArtifactError::StorageNotConfigured);
    }
    let req = read_publish(multipart).await?;
    let response = super::service::publish(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &task_id,
        req,
    )
    .await?;
    Ok(Json(response))
}

/// List a Task's published Artifacts, most recently updated first.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/artifacts",
    tag = "artifacts",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Artifact list", body = ListArtifactsResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_task_artifacts(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<ListArtifactsResponse>, ArtifactError> {
    let items = super::service::list(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &task_id,
    )
    .await?;
    Ok(Json(ListArtifactsResponse { items }))
}

/// Redirect to the page of the Artifact's latest version.
#[utoipa::path(
    get,
    path = "/api/v1/artifacts/{artifact_id}",
    tag = "artifacts",
    params(
        ("artifact_id" = String, Path, description = "Artifact identifier")
    ),
    responses(
        (status = 302, description = "Redirect to the latest version"),
        (status = 404, description = "Artifact not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn open_latest_artifact(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(artifact_id): Path<String>,
) -> Result<Response, ArtifactError> {
    let location = super::service::latest_location(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
    )
    .await?;
    let location = HeaderValue::try_from(location)
        .map_err(|_| ArtifactError::NotFound(artifact_id.clone()))?;
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
    )
        .into_response())
}

/// Serve the page of a published Artifact version.
#[utoipa::path(
    get,
    path = "/api/v1/artifacts/{artifact_id}/{version}",
    tag = "artifacts",
    params(
        ("artifact_id" = String, Path, description = "Artifact identifier"),
        ("version" = String, Path, description = "Version, as in `v3`")
    ),
    responses(
        (status = 200, description = "The HTML page", content_type = "text/html"),
        (status = 404, description = "Artifact or version not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn serve_artifact_page(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path((artifact_id, version)): Path<(String, String)>,
) -> Result<Response, ArtifactError> {
    let bytes = super::service::read_page(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
        &version,
    )
    .await?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(PAGE_CONTENT_TYPE),
            ),
            // Agent-written and served from the application's own origin. No
            // `allow-same-origin`, so it renders in an opaque one.
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("sandbox allow-scripts allow-forms allow-popups"),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// Collect the multipart form, refusing it as soon as the page passes
/// [`MAX_PAGE_BYTES`] rather than after buffering all of it.
async fn read_publish(mut multipart: Multipart) -> Result<PublishRequest, ArtifactError> {
    let mut req = PublishRequest::default();
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name().unwrap_or_default() {
            "title" => req.title = field.text().await.map_err(multipart_error)?,
            "slug" => req.slug = Some(field.text().await.map_err(multipart_error)?),
            "file" => {
                if req.file_name.is_some() {
                    return Err(ArtifactError::BadRequest(
                        "an artifact is a single HTML file; send one `file` part".to_string(),
                    ));
                }
                let name = field.file_name().unwrap_or_default().to_string();
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    if req.html.len() + chunk.len() > MAX_PAGE_BYTES {
                        return Err(ArtifactError::PayloadTooLarge(format!(
                            "{name} exceeds {MAX_PAGE_BYTES} bytes"
                        )));
                    }
                    req.html.extend_from_slice(&chunk);
                }
                req.file_name = Some(name);
            }
            other => {
                return Err(ArtifactError::BadRequest(format!(
                    "unexpected form field {other:?}"
                )));
            }
        }
    }
    Ok(req)
}

fn multipart_error(e: MultipartError) -> ArtifactError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ArtifactError::PayloadTooLarge(e.body_text())
    } else {
        ArtifactError::BadRequest(format!("multipart error: {}", e.body_text()))
    }
}
