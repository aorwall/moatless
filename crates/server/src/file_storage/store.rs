//! Persisting one upload: the bytes into S3, the metadata into Postgres, and
//! the removal that undoes both.
//!
//! Three surfaces reach this. The multipart route takes a form field; the T3
//! compatibility surface takes an image that arrived inline on a turn, because
//! a client that does not know this server accepts uploads sends attachment
//! bytes in the command instead; and the T3 attachment-upload route takes a
//! raw body under an id it minted beforehand. They differ only in how the bytes
//! got here, so what an upload *is* — a uuid, an object under the configured
//! prefix, and a metadata row scoped to its creator — is decided once, and all
//! three are readable afterwards through `GET /api/v1/uploads/{file_id}`.

use moatless_core::config::S3Config;
use moatless_types::access_control::AccessControlError;
use moatless_types::access_control::resolve_scope_for_create;
use moatless_types::access_control::{Scope, ScopeContext, ScopedResource};

use super::dao::FileDao;
use super::error::FileStorageError;
use super::models::FileMetadata;
use super::s3;

/// One file's bytes, and what the sender called them.
pub struct NewUpload {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
    /// The id to store under, when the caller already handed one out. The T3
    /// attachment flow mints the id before the bytes move — the client holds it
    /// while the upload is in flight and names it on the turn afterwards — so
    /// that id has to be the one the row is written under. `None` mints a fresh
    /// uuid here, which is what both of the other two surfaces want.
    pub file_id: Option<String>,
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

    let file_id = upload
        .file_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
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
    .map_err(FileStorageError::from)?;

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

    // The object is already in S3 and its id has not been returned to anyone,
    // so a failed row would leave bytes no caller can name and none of them can
    // clean up. Undo the put before surfacing the error, and report the row
    // failure rather than the compensation's.
    if let Err(error) = files.insert_metadata(&meta).await {
        if let Err(cleanup) = s3::delete_object(&s3_client.client, bucket, &s3_key).await {
            tracing::warn!(%cleanup, file_id = %meta.file_id, "Upload object outlived the row that failed");
        }
        return Err(error.into());
    }

    Ok(meta)
}

/// Remove an upload: its object, then its metadata row. Answers whether there
/// was one to remove.
///
/// An id that names nothing is `Ok(false)` rather than an error — the T3
/// composer deletes an attachment it abandoned without knowing whether the
/// bytes ever landed, and that is not a failure to report to anyone.
///
/// The object goes first. The row is what every other surface reaches an upload
/// through, so a failure between the two leaves an unreferenced object rather
/// than a row pointing at bytes that are gone.
///
/// Access is all this checks. Whether anything still points at the upload is a
/// question about the caller's own domain, not about file storage: the T3
/// attachment store asks it in
/// `crate::t3code::backend::AppStateAttachments::is_on_a_message` before it
/// deletes or replaces anything, so a sent attachment is not collected as an
/// abandoned one.
pub async fn delete_upload(
    s3_storage: Option<&s3::S3Storage>,
    s3_config: &S3Config,
    files: &dyn FileDao,
    viewer: &ScopeContext,
    file_id: &str,
) -> Result<bool, FileStorageError> {
    let s3_client = s3_storage.ok_or(FileStorageError::S3NotConfigured)?;
    let bucket = s3_config
        .bucket_name
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;

    let Some(meta) = files.find_by_id(file_id).await? else {
        return Ok(false);
    };
    authorize_read(viewer, &meta)?;

    let s3_key = format!("{}/{}", s3_config.key_prefix, file_id);
    s3::delete_object(&s3_client.client, bucket, &s3_key)
        .await
        .map_err(FileStorageError::from)?;

    files.delete_metadata(file_id).await?;

    Ok(true)
}

/// File bytes and the metadata needed by the HTTP download adapter.
pub struct Download {
    pub data: Vec<u8>,
    pub content_type: String,
    pub filename: String,
}

/// Read an upload after enforcing its metadata access policy.
pub async fn read_upload(
    s3_storage: Option<&s3::S3Storage>,
    s3_config: &S3Config,
    files: &dyn FileDao,
    viewer: &ScopeContext,
    file_id: &str,
) -> Result<Download, FileStorageError> {
    let s3_client = s3_storage.ok_or(FileStorageError::S3NotConfigured)?;
    let bucket = s3_config
        .bucket_name
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;
    let meta = files
        .find_by_id(file_id)
        .await?
        .ok_or_else(|| FileStorageError::NotFound(file_id.to_owned()))?;
    authorize_read(viewer, &meta)?;
    let s3_key = format!("{}/{}", s3_config.key_prefix, file_id);
    let (data, content_type) = s3::get_object(&s3_client.client, bucket, &s3_key).await?;
    Ok(Download {
        data,
        content_type,
        filename: meta.filename,
    })
}

fn authorize_read(viewer: &ScopeContext, meta: &FileMetadata) -> Result<(), FileStorageError> {
    let resource = ScopedResource::borrowed(
        meta.scope,
        meta.owner_user_id.as_deref().or(meta.created_by.as_deref()),
    );
    if !viewer.can_read(&resource) {
        return Err(FileStorageError::Forbidden(format!(
            "No access to file {}",
            meta.file_id
        )));
    }

    Ok(())
}

#[cfg(test)]
mod access_tests {
    use super::*;

    #[test]
    fn download_and_delete_share_owner_fallback_and_global_access() {
        let mut file = FileMetadata {
            file_id: "upload".into(), filename: "attachment.png".into(),
            mime_type: "image/png".into(), size: 3, scope: Scope::User,
            owner_user_id: None, created_by: Some("creator".into()),
            uploaded_at: chrono::Utc::now(),
        };
        let creator = ScopeContext { user_id: "creator".into(), is_admin: false };
        let stranger = ScopeContext { user_id: "stranger".into(), is_admin: false };
        assert!(authorize_read(&creator, &file).is_ok());
        assert!(matches!(authorize_read(&stranger, &file), Err(FileStorageError::Forbidden(_))));
        file.owner_user_id = Some("owner".into());
        assert!(authorize_read(&creator, &file).is_err());
        file.scope = Scope::Global;
        assert!(authorize_read(&stranger, &file).is_ok());
    }
}
