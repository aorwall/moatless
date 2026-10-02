//! The `task_artifacts` table and the S3 objects behind it, as a host and the
//! artifact routes reach them.

use std::sync::Arc;

use moatless_core::config::S3Config;
use moatless_s3::S3Storage;

use super::dao::ArtifactDao;
use super::error::ArtifactError;
use super::models::{Artifact, Reservation};

/// Content type a page is stored and served with, whatever the upload claimed.
pub(crate) const PAGE_CONTENT_TYPE: &str = "text/html; charset=utf-8";

/// Artifact rows and their objects in S3.
///
/// Validation and the access check stay in [`crate::artifact::service`]. Every
/// operation touching bytes answers [`ArtifactError::StorageNotConfigured`] on
/// a deployment without object storage.
pub struct Artifacts {
    store: Arc<dyn ArtifactDao>,
    s3: Option<S3Storage>,
    s3_config: Arc<S3Config>,
    /// Origin the absolute URLs in a response start with, without a trailing
    /// slash.
    public_url: String,
}

impl Artifacts {
    pub(crate) fn new(
        store: Arc<dyn ArtifactDao>,
        s3: Option<S3Storage>,
        s3_config: Arc<S3Config>,
        public_url: &str,
    ) -> Self {
        Self {
            store,
            s3,
            s3_config,
            public_url: public_url.trim_end_matches('/').to_string(),
        }
    }

    fn object_storage(&self) -> Result<(&S3Storage, &str), ArtifactError> {
        let client = self
            .s3
            .as_ref()
            .ok_or(ArtifactError::StorageNotConfigured)?;
        let bucket = self
            .s3_config
            .bucket_name
            .as_deref()
            .ok_or(ArtifactError::StorageNotConfigured)?;
        Ok((client, bucket))
    }

    /// Whether this deployment can store Artifacts, for a caller that would
    /// otherwise buffer a request body only to be refused.
    pub fn is_configured(&self) -> bool {
        self.object_storage().is_ok()
    }

    pub(crate) fn public_url(&self) -> &str {
        &self.public_url
    }

    /// Key of one version's page.
    fn page_key(&self, task_id: &str, artifact_id: &str, version: i32) -> String {
        format!(
            "{}/{task_id}/{artifact_id}/v{version}.html",
            self.s3_config.artifact_prefix
        )
    }

    /// Upload `html` as `version`'s page.
    pub(crate) async fn put_page(
        &self,
        task_id: &str,
        artifact_id: &str,
        version: i32,
        html: Vec<u8>,
    ) -> Result<(), ArtifactError> {
        let (s3, bucket) = self.object_storage()?;
        let key = self.page_key(task_id, artifact_id, version);
        moatless_s3::put_object(&s3.client, bucket, &key, html, PAGE_CONTENT_TYPE).await?;
        Ok(())
    }

    /// Best-effort removal of one version's page, for a publish that did not
    /// complete.
    pub(crate) async fn delete_page(&self, task_id: &str, artifact_id: &str, version: i32) {
        let Ok((s3, bucket)) = self.object_storage() else {
            return;
        };
        let key = self.page_key(task_id, artifact_id, version);
        if let Err(e) = moatless_s3::delete_object(&s3.client, bucket, &key).await {
            tracing::warn!(
                error = %moatless_core::error::causal_chain(&e),
                %key,
                "Failed to delete the page of an incomplete artifact version"
            );
        }
    }

    /// One version's page, or `None` when it has no object.
    pub(crate) async fn read_page(
        &self,
        task_id: &str,
        artifact_id: &str,
        version: i32,
    ) -> Result<Option<Vec<u8>>, ArtifactError> {
        let (s3, bucket) = self.object_storage()?;
        let key = self.page_key(task_id, artifact_id, version);
        Ok(moatless_s3::get_object_opt(&s3.client, bucket, &key).await?)
    }

    /// See [`ArtifactDao::reserve_version`].
    pub(crate) async fn reserve_version(
        &self,
        new_artifact_id: &str,
        task_id: &str,
        slug: &str,
        title: &str,
        created_by: &str,
    ) -> Result<Reservation, sqlx::Error> {
        self.store
            .reserve_version(new_artifact_id, task_id, slug, title, created_by)
            .await
    }

    /// See [`ArtifactDao::commit_version`].
    pub(crate) async fn commit_version(
        &self,
        artifact_id: &str,
        version: i32,
        title: &str,
    ) -> Result<Option<Artifact>, sqlx::Error> {
        self.store.commit_version(artifact_id, version, title).await
    }

    /// See [`ArtifactDao::find_by_id`].
    pub(crate) async fn find_by_id(
        &self,
        artifact_id: &str,
    ) -> Result<Option<Artifact>, sqlx::Error> {
        self.store.find_by_id(artifact_id).await
    }

    /// See [`ArtifactDao::list_for_task`].
    pub(crate) async fn list_for_task(&self, task_id: &str) -> Result<Vec<Artifact>, sqlx::Error> {
        self.store.list_for_task(task_id).await
    }
}
