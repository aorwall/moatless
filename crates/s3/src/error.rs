/// The SDK error behind a failed call, kept for its message and source chain.
type Cause = Box<dyn std::error::Error + Send + Sync>;

/// An object-storage or signing failure with its original cause.
#[derive(Debug, thiserror::Error)]
pub enum S3Error {
    #[error("S3 put_object failed: {0}")]
    PutObject(#[source] Cause),
    #[error("S3 get_object failed: {0}")]
    GetObject(#[source] Cause),
    #[error("Failed to read S3 body: {0}")]
    ReadBody(#[source] Cause),
    #[error("S3 head_object failed: {0}")]
    HeadObject(#[source] Cause),
    #[error("S3 delete_object failed: {0}")]
    DeleteObject(#[source] Cause),
    #[error("S3 list_objects_v2 failed: {0}")]
    ListObjects(#[source] Cause),
    #[error("S3 delete_objects failed: {0}")]
    DeleteObjects(#[source] Cause),
    #[error("S3 delete identifier invalid: {0}")]
    InvalidDeleteIdentifier(#[source] Cause),
    #[error("S3 delete request invalid: {0}")]
    InvalidDeleteRequest(#[source] Cause),
    #[cfg(feature = "sts")]
    #[error("STS assume_role failed: {0}")]
    AssumeRole(#[source] Cause),
    #[cfg(feature = "sts")]
    #[error("STS assume_role returned no credentials")]
    SessionWithoutKeys,
    #[cfg(feature = "sts")]
    #[error("STS session expiry out of range")]
    SessionExpiryOutOfRange,
    #[error("S3 range read of {key} asked for zero bytes")]
    EmptyRange { key: String },
    #[error("S3 head_object returned no LastModified for {key}")]
    MissingLastModified { key: String },
    #[error("S3 LastModified out of range for {key}")]
    InvalidLastModified { key: String },
    #[error("presign config invalid: {0}")]
    InvalidPresignConfig(#[source] Cause),
    #[error("S3 presign put_object failed: {0}")]
    PresignPut(#[source] Cause),
    #[error("S3 presign get_object failed: {0}")]
    PresignGet(#[source] Cause),
    #[error("S3 presign delete_object failed: {0}")]
    PresignDelete(#[source] Cause),
    #[error("failed to load AWS credentials for presign TTL: {0}")]
    Credentials(#[source] Cause),
    #[error("AWS credentials expired before S3 presign")]
    CredentialsExpired,
    #[error("AWS credentials expire too soon for S3 presign")]
    CredentialsExpiring,
    #[error("S3 delete_objects left {} of {attempted} keys: {}", failures.len(), first_failure(failures))]
    BatchDelete {
        attempted: usize,
        failures: Vec<DeleteFailure>,
    },
}

/// One key a `delete_objects` batch reported it could not delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteFailure {
    pub key: Option<String>,
    /// The S3 error code, such as `AccessDenied`.
    pub code: Option<String>,
    pub message: Option<String>,
}

impl DeleteFailure {
    pub(crate) fn from_sdk(error: &aws_sdk_s3::types::Error) -> Self {
        Self {
            key: error.key().map(str::to_string),
            code: error.code().map(str::to_string),
            message: error.message().map(str::to_string),
        }
    }
}

fn first_failure(failures: &[DeleteFailure]) -> String {
    failures.first().map_or_default(|failure| {
        format!(
            "{}: {}",
            failure.key.as_deref().unwrap_or("?"),
            failure.message.as_deref().unwrap_or("?")
        )
    })
}
