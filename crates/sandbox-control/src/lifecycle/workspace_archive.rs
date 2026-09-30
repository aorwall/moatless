//! [`WorkspaceArchivePort`] over the S3 workspace archive.
//!
//! The cold transition trades a live volume for the snapshot at
//! `<prefix>/<task_id>/workspace.tar` — the same object the sandbox pushes to,
//! addressed through the same key builder so a divergence here could not
//! silently head the wrong object.
//!
//! Answers when the snapshot was last written, and hands a presigned upload URL
//! to whatever workload the runtime schedules to read the detached volume,
//! keeping AWS credentials out of it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use moatless_s3::{self as file_s3, S3Storage};

use super::orchestrator::WorkspaceArchivePort;
use moatless_sandbox_runtime::storage::sandbox_key;

/// Lifetime of a rescue upload URL.
///
/// Longer than the sandbox's own 15-minute sync TTL: the holder starts from
/// nothing (scheduled, image pulled, claim mounted) before the first byte
/// moves, and must still be valid when the last one does.
const RESCUE_PRESIGN_TTL: Duration = Duration::from_mins(30);

/// Reads (and hands out write capabilities for) the S3 workspace archive.
pub struct S3WorkspaceArchive {
    s3: Arc<S3Storage>,
    bucket: String,
    /// `config.s3.workspace_prefix` — the same prefix `GetWorkspaceSyncUrls`
    /// mints against.
    prefix: String,
}

impl S3WorkspaceArchive {
    pub fn new(s3: Arc<S3Storage>, bucket: String, prefix: String) -> Self {
        Self { s3, bucket, prefix }
    }

    fn key(&self, task_id: &str) -> String {
        sandbox_key(&self.prefix, task_id)
    }
}

#[async_trait]
impl WorkspaceArchivePort for S3WorkspaceArchive {
    async fn archived_at(&self, task_id: &str) -> Result<Option<DateTime<Utc>>, String> {
        file_s3::head_object_modified_at(&self.s3, &self.bucket, &self.key(task_id))
            .await
            .map_err(|error| error.to_string())
    }

    async fn presign_upload(&self, task_id: &str) -> Result<String, String> {
        file_s3::presign_put(
            &self.s3,
            &self.bucket,
            &self.key(task_id),
            RESCUE_PRESIGN_TTL,
        )
        .await
        .map(|presigned| presigned.url)
        .map_err(|error| error.to_string())
    }
}
