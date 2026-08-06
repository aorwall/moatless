//! File storage HTTP handlers.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::Response;
use axum::Json;
use axum_extra::extract::Multipart;

use std::sync::Arc;

use super::models::FileMetadata;
use moatless_backend_core::access_control::resolve_scope_for_create;
use moatless_core::config::S3Config;
use crate::extractors::AuthUser;
use moatless_backend_core::scope::ScopedResource;

use super::dao::FileDao;
use super::error::FileStorageError;
use super::s3;
use super::schemas::UploadResponse;

/// Upload a file via multipart form data.
#[utoipa::path(
    post,
    path = "/api/v1/uploads",
    tag = "file-storage",
    request_body(content_type = "multipart/form-data", content = super::schemas::UploadFileBody),
    responses(
        (status = 200, description = "File uploaded successfully", body = UploadResponse),
        (status = 400, description = "Bad request", body = moatless_core::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn upload_file(
    State(s3_storage): State<Option<s3::S3Storage>>,
    State(s3_config): State<Arc<S3Config>>,
    State(files): State<Arc<dyn FileDao>>,
    auth: AuthUser,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, FileStorageError> {
    let s3_client = s3_storage
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;

    let bucket = s3_config
        .bucket_name
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;

    let mut file_data = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| FileStorageError::BadRequest(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let filename = field.file_name().unwrap_or("unnamed").to_string();
            let content_type = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();
            let data = field
                .bytes()
                .await
                .map_err(|e| FileStorageError::UploadFailed(format!("Read error: {e}")))?;

            file_data = Some((filename, content_type, data.to_vec()));
        }
    }

    let (filename, content_type, data) =
        file_data.ok_or_else(|| FileStorageError::BadRequest("No file field found".to_string()))?;

    let file_id = uuid::Uuid::new_v4().to_string();
    let s3_key = format!("{}/{}", s3_config.key_prefix, file_id);
    let resolved_scope =
        resolve_scope_for_create(&auth.scope_context(), Some(moatless_backend_core::scope::Scope::User))
            .map_err(|e| match e {
                moatless_backend_core::access_control::AccessControlError::BadRequest(msg) => {
                    FileStorageError::BadRequest(msg)
                }
                moatless_backend_core::access_control::AccessControlError::Forbidden(msg) => {
                    FileStorageError::Forbidden(msg)
                }
            })?;

    // Upload to S3
    s3::put_object(
        &s3_client.client,
        bucket,
        &s3_key,
        data.clone(),
        &content_type,
    )
    .await
    .map_err(FileStorageError::UploadFailed)?;

    // Save metadata
    let meta = FileMetadata {
        file_id: file_id.clone(),
        filename: filename.clone(),
        mime_type: content_type.clone(),
        size: data.len() as i64,
        scope: resolved_scope.scope,
        owner_user_id: resolved_scope.owner_user_id,
        created_by: Some(auth.user_id.to_string()),
        uploaded_at: chrono::Utc::now(),
    };

    files.insert_metadata(&meta).await?;

    Ok(Json(UploadResponse {
        file_id,
        filename,
        mime_type: content_type,
        size: meta.size,
    }))
}

/// Download a file by ID.
#[utoipa::path(
    get,
    path = "/api/v1/uploads/{file_id}",
    tag = "file-storage",
    params(
        ("file_id" = String, Path, description = "File identifier")
    ),
    responses(
        (status = 200, description = "File content", content_type = "application/octet-stream"),
        (status = 404, description = "File not found", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn download_file(
    State(s3_storage): State<Option<s3::S3Storage>>,
    State(s3_config): State<Arc<S3Config>>,
    State(files): State<Arc<dyn FileDao>>,
    auth: AuthUser,
    Path(file_id): Path<String>,
) -> Result<Response, FileStorageError> {
    let s3_client = s3_storage
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;

    let bucket = s3_config
        .bucket_name
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;

    let meta = files
        .find_by_id(&file_id)
        .await?
        .ok_or_else(|| FileStorageError::NotFound(file_id.clone()))?;
    let resource = ScopedResource::borrowed(
        meta.scope,
        meta.owner_user_id.as_deref().or(meta.created_by.as_deref()),
    );
    if !auth.scope_context().can_read(&resource) {
        return Err(FileStorageError::Forbidden(format!(
            "No access to file {file_id}"
        )));
    }

    let s3_key = format!("{}/{}", s3_config.key_prefix, file_id);
    let (data, content_type) = s3::get_object(&s3_client.client, bucket, &s3_key)
        .await
        .map_err(FileStorageError::UploadFailed)?;

    #[allow(clippy::expect_used)]
    // Response::builder() only fails with invalid header names (ours are static)
    let response = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", meta.filename),
        )
        .body(Body::from(data))
        .expect("valid response with static header names");

    Ok(response)
}
