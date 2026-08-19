//! Persisting one upload: the bytes into S3, the metadata into Postgres.
//!
//! Two surfaces reach this. The multipart route takes a form field; the T3
//! compatibility surface takes an image that arrived inline on a turn, because
//! that client sends attachment bytes in the command instead of uploading them
//! first. They differ only in how the bytes got here, so what an upload *is* —
//! a uuid, an object under the configured prefix, and a metadata row scoped to
//! its creator — is decided once, and both are readable afterwards through
//! `GET /api/v1/uploads/{file_id}`.

use moatless_backend_core::access_control::resolve_scope_for_create;
use moatless_types::access_control::AccessControlError;
use moatless_types::access_control::{Scope, ScopeContext};
use moatless_core::config::S3Config;

use super::dao::FileDao;
use super::error::FileStorageError;
use super::models::FileMetadata;
use super::s3;

/// One file's bytes, and what the sender called them.
pub struct NewUpload {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// Store `upload` and return the metadata row written for it.
///
/// The row is `User` scoped and owned by `viewer`, which is what makes the
/// download route's `can_read` check the thing that decides who may fetch it
/// back.
pub async fn store_upload(
    s3_storage: Option<&s3::S3Storage>,
    s3_config: &S3Config,
    files: &dyn FileDao,
    viewer: &ScopeContext,
    upload: NewUpload,
) -> Result<FileMetadata, FileStorageError> {
    let s3_client = s3_storage.ok_or(FileStorageError::S3NotConfigured)?;
    let bucket = s3_config
        .bucket_name
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;

    let resolved_scope =
        resolve_scope_for_create(viewer, Some(Scope::User)).map_err(|e| match e {
            AccessControlError::BadRequest(msg) => FileStorageError::BadRequest(msg),
            AccessControlError::Forbidden(msg) => FileStorageError::Forbidden(msg),
        })?;

    let file_id = uuid::Uuid::new_v4().to_string();
    let s3_key = format!("{}/{}", s3_config.key_prefix, file_id);
    // Read before the bytes are moved into the upload; the row records what was
    // actually stored rather than what the sender said the size was.
    let size = upload.data.len() as i64;

    s3::put_object(
        &s3_client.client,
        bucket,
        &s3_key,
        upload.data,
        &upload.mime_type,
    )
    .await
    .map_err(FileStorageError::UploadFailed)?;

    let meta = FileMetadata {
        file_id,
        filename: upload.filename,
        mime_type: upload.mime_type,
        size,
        scope: resolved_scope.scope,
        owner_user_id: resolved_scope.owner_user_id,
        created_by: Some(viewer.user_id.clone()),
        uploaded_at: chrono::Utc::now(),
    };

    files.insert_metadata(&meta).await?;

    Ok(meta)
}
