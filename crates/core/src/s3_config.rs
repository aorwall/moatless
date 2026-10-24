//! The S3 bucket, region and key prefixes the backend stores files under.

/// S3 / file storage settings.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// S3 bucket name for file uploads.
    pub bucket_name: Option<String>,
    /// S3 key prefix for uploaded files (default: "uploads")
    pub key_prefix: String,
    /// S3 key prefix for sandbox Claude transcripts, stored under
    /// `<prefix>/<task_id>/transcripts.tar` (default "transcripts").
    ///
    /// Reuses the uploads bucket — the backend IRSA already has read/write on
    /// it. Env `TRANSCRIPT_S3_PREFIX`.
    pub transcript_prefix: String,
    /// S3 key prefix for the `s3` storage backend's workspace archives, under
    /// `<prefix>/<task_id>/workspace.tar`. Reuses the uploads bucket.
    ///
    /// The sandbox never gets S3 credentials — the backend presigns PUT/GET
    /// URLs scoped to this prefix. Env `WORKSPACE_S3_PREFIX` (default
    /// "workspaces").
    pub workspace_prefix: String,
    /// S3 key prefix for published Artifacts, under
    /// `<prefix>/<task_id>/<artifact_id>/v<n>.html`. Reuses the uploads
    /// bucket. Env `ARTIFACT_S3_PREFIX` (default "artifacts").
    pub artifact_prefix: String,
    /// S3 region (default: "us-east-1")
    pub region: String,
}
