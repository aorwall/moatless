//! Uploads as a caller reaches them: the bytes into S3, the metadata into
//! Postgres, and the removal that undoes both.
//!
//! Three surfaces store an upload. The multipart route takes a form field; the
//! T3 compatibility surface takes an image that arrived inline on a turn,
//! because a client that does not know this server accepts uploads sends
//! attachment bytes in the command instead; and the T3 attachment-upload route
//! takes a raw body under an id it minted beforehand. They differ only in how
//! the bytes got here, so what an upload *is* — a uuid, an object under the
//! configured prefix, and a metadata row scoped to its creator — is decided
//! once, and all three are readable afterwards through
//! `GET /api/v1/uploads/{file_id}`.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_s3::S3Storage;
use moatless_types::access_control::AccessControlError;
use moatless_types::access_control::resolve_scope_for_create;
use moatless_types::access_control::{Scope, ScopeContext, ScopedResource};

use super::dao::FileDao;
use super::error::FileStorageError;
use super::models::{Attachment, Download, FileInfo, FileMetadata, NewUpload};
use crate::config::S3Config;

/// The uploads a caller may reach, over the object storage they live in.
///
/// Built by [`super::compose::files`]. Every operation that touches bytes
/// answers [`FileStorageError::S3NotConfigured`] on a deployment without object
/// storage; the metadata reads answer without it.
pub struct Files {
    store: Arc<dyn FileDao>,
    s3: Option<S3Storage>,
    s3_config: Arc<S3Config>,
}

impl Files {
    pub(super) fn new(
        store: Arc<dyn FileDao>,
        s3: Option<S3Storage>,
        s3_config: Arc<S3Config>,
    ) -> Self {
        Self {
            store,
            s3,
            s3_config,
        }
    }

    /// The client and bucket an operation over bytes needs.
    fn object_storage(&self) -> Result<(&S3Storage, &str), FileStorageError> {
        let client = self.s3.as_ref().ok_or(FileStorageError::S3NotConfigured)?;
        let bucket = self
            .s3_config
            .bucket_name
            .as_deref()
            .ok_or(FileStorageError::S3NotConfigured)?;
        Ok((client, bucket))
    }

    /// Whether this deployment can store bytes at all, for a caller that would
    /// otherwise buffer a request body only to be refused.
    pub fn is_configured(&self) -> bool {
        self.object_storage().is_ok()
    }

    fn key(&self, file_id: &str) -> String {
        format!("{}/{}", self.s3_config.key_prefix, file_id)
    }

    /// Store `upload` and return what was written for it.
    ///
    /// The row is `User` scoped and owned by `viewer`, which is what makes
    /// [`Files::read`]'s access check the thing that decides who may fetch it
    /// back.
    pub async fn store_upload(
        &self,
        viewer: &ScopeContext,
        upload: NewUpload,
    ) -> Result<FileInfo, FileStorageError> {
        let (s3_client, bucket) = self.object_storage()?;

        let resolved_scope =
            resolve_scope_for_create(viewer, Some(Scope::User)).map_err(|e| match e {
                AccessControlError::BadRequest(msg) => FileStorageError::BadRequest(msg),
                AccessControlError::Forbidden(msg) => FileStorageError::Forbidden(msg),
            })?;

        let file_id = upload
            .file_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let s3_key = self.key(&file_id);
        // Read before the bytes are moved into the upload; the row records what
        // was actually stored rather than what the sender said the size was.
        let size = upload.data.len() as i64;

        moatless_s3::put_object(
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

        // The object is already in S3 and its id has not been returned to
        // anyone, so a failed row would leave bytes no caller can name and none
        // of them can clean up. Undo the put before surfacing the error, and
        // report the row failure rather than the compensation's.
        if let Err(error) = self.store.insert_metadata(&meta).await {
            if let Err(cleanup) =
                moatless_s3::delete_object(&s3_client.client, bucket, &s3_key).await
            {
                tracing::warn!(%cleanup, file_id = %meta.file_id, "Upload object outlived the row that failed");
            }
            return Err(error.into());
        }

        Ok(meta.info())
    }

    /// Read an upload after enforcing its metadata access policy.
    pub async fn read(
        &self,
        viewer: &ScopeContext,
        file_id: &str,
    ) -> Result<Download, FileStorageError> {
        let (s3_client, bucket) = self.object_storage()?;
        let meta = self
            .store
            .find_by_id(file_id)
            .await?
            .ok_or_else(|| FileStorageError::NotFound(file_id.to_owned()))?;
        authorize_read(viewer, &meta)?;
        let (data, content_type) =
            moatless_s3::get_object(&s3_client.client, bucket, &self.key(file_id)).await?;
        Ok(Download {
            data,
            content_type,
            filename: meta.filename,
        })
    }

    /// Remove an upload: its object, then its metadata row. Answers whether
    /// there was one to remove.
    ///
    /// An id that names nothing is `Ok(false)` rather than an error — the T3
    /// composer deletes an attachment it abandoned without knowing whether the
    /// bytes ever landed, and that is not a failure to report to anyone.
    ///
    /// The object goes first and the row second. A failure between the two
    /// leaves the row naming bytes that are gone, and a later [`Self::read`] of
    /// that id reads the row and then fails on the fetch. Calling `delete`
    /// again clears it: S3 answers a delete of an absent key with success.
    ///
    /// Access is all this checks. Whether anything still points at the upload
    /// is a question about the caller's own domain, not about file storage: the
    /// T3 attachment store asks it in
    /// `moatless_backend::t3code::backend::AppStateAttachments::is_on_a_message`
    /// before it deletes or replaces anything, so a sent attachment is not
    /// collected as an abandoned one.
    pub async fn delete(
        &self,
        viewer: &ScopeContext,
        file_id: &str,
    ) -> Result<bool, FileStorageError> {
        let (s3_client, bucket) = self.object_storage()?;

        let Some(meta) = self.store.find_by_id(file_id).await? else {
            return Ok(false);
        };
        authorize_read(viewer, &meta)?;

        moatless_s3::delete_object(&s3_client.client, bucket, &self.key(file_id))
            .await
            .map_err(FileStorageError::from)?;

        self.store.delete_metadata(file_id).await?;

        Ok(true)
    }

    /// What an upload is called, without its bytes and without a viewer: the
    /// prompt and transcript surfaces render a name and a size for an id a row
    /// they already read gave them.
    pub async fn metadata(&self, file_id: &str) -> Result<Option<FileInfo>, FileStorageError> {
        Ok(self
            .store
            .find_by_id(file_id)
            .await?
            .map(|meta| meta.info()))
    }

    /// [`Files::metadata`] under a viewer, for a caller that has to decide on
    /// an upload without reading its bytes. Refuses exactly as [`Files::read`]
    /// does; an id that names nothing is `Ok(None)`.
    pub async fn readable_metadata(
        &self,
        viewer: &ScopeContext,
        file_id: &str,
    ) -> Result<Option<FileInfo>, FileStorageError> {
        let Some(meta) = self.store.find_by_id(file_id).await? else {
            return Ok(None);
        };
        authorize_read(viewer, &meta)?;
        Ok(Some(meta.info()))
    }

    /// [`Files::metadata`] for a page of ids, in one query.
    pub async fn metadata_for(
        &self,
        file_ids: &[String],
    ) -> Result<HashMap<String, FileInfo>, FileStorageError> {
        Ok(self
            .store
            .find_by_ids(file_ids)
            .await?
            .into_iter()
            .map(|(id, meta)| (id, meta.info()))
            .collect())
    }

    /// The uploads an outbound message carries, bytes included.
    ///
    /// An id that resolves to nothing is dropped with a warning rather than
    /// failing the send: the message is worth delivering without an attachment
    /// that is already gone. A deployment without object storage resolves
    /// nothing at all.
    pub async fn resolve_attachments(&self, file_ids: &[String]) -> Vec<Attachment> {
        if file_ids.is_empty() {
            return vec![];
        }
        let Ok((s3_client, bucket)) = self.object_storage() else {
            tracing::warn!("S3 not configured, cannot resolve file attachments");
            return vec![];
        };

        let mut resolved = Vec::with_capacity(file_ids.len());
        for file_id in file_ids {
            let meta = match self.store.find_by_id(file_id).await {
                Ok(Some(m)) => m,
                Ok(None) => {
                    tracing::warn!(file_id = %file_id, "File metadata not found for outbound attachment");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(file_id = %file_id, error = %e, "Failed to load file metadata");
                    continue;
                }
            };

            match moatless_s3::get_object(&s3_client.client, bucket, &self.key(file_id)).await {
                Ok((data, _content_type)) => resolved.push(Attachment {
                    filename: meta.filename,
                    mime_type: meta.mime_type,
                    data,
                }),
                Err(e) => {
                    tracing::warn!(file_id = %file_id, error = %e, "Failed to download file from S3");
                }
            }
        }
        resolved
    }
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
            file_id: "upload".into(),
            filename: "attachment.png".into(),
            mime_type: "image/png".into(),
            size: 3,
            scope: Scope::User,
            owner_user_id: None,
            created_by: Some("creator".into()),
            uploaded_at: chrono::Utc::now(),
        };
        let creator = ScopeContext {
            user_id: "creator".into(),
            is_admin: false,
        };
        let stranger = ScopeContext {
            user_id: "stranger".into(),
            is_admin: false,
        };
        assert!(authorize_read(&creator, &file).is_ok());
        assert!(matches!(
            authorize_read(&stranger, &file),
            Err(FileStorageError::Forbidden(_))
        ));
        file.owner_user_id = Some("owner".into());
        assert!(authorize_read(&creator, &file).is_err());
        file.scope = Scope::Global;
        assert!(authorize_read(&stranger, &file).is_ok());
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::super::test_support::files_from;
    use super::*;

    fn info(file_id: &str, filename: &str) -> FileInfo {
        FileInfo {
            file_id: file_id.into(),
            filename: filename.into(),
            mime_type: "text/plain".into(),
            size: 4,
        }
    }

    #[tokio::test]
    async fn metadata_reads_one_row_and_a_page_of_them() {
        let files = files_from(vec![info("a", "one.txt"), info("b", "two.txt")]);

        assert_eq!(
            files.metadata("a").await.unwrap().unwrap().filename,
            "one.txt"
        );
        assert!(files.metadata("missing").await.unwrap().is_none());

        let page = files
            .metadata_for(&["a".to_string(), "missing".to_string()])
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page["a"].mime_type, "text/plain");
    }

    /// Without object storage there is nothing to resolve, and the send that
    /// asked goes out without the attachment rather than failing.
    #[tokio::test]
    async fn attachments_resolve_to_nothing_without_object_storage() {
        let files = files_from(vec![info("a", "one.txt")]);

        let viewer = ScopeContext {
            user_id: "reader".into(),
            is_admin: false,
        };
        assert!(
            files
                .resolve_attachments(&["a".to_string()])
                .await
                .is_empty()
        );
        assert!(matches!(
            files.read(&viewer, "a").await,
            Err(FileStorageError::S3NotConfigured)
        ));
    }
}
