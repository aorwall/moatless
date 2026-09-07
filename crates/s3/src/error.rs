use aws_sdk_s3::error::{BoxError, BuildError, SdkError};
use aws_sdk_s3::operation::{
    delete_object::DeleteObjectError, delete_objects::DeleteObjectsError,
    get_object::GetObjectError, head_object::HeadObjectError, list_objects_v2::ListObjectsV2Error,
    put_object::PutObjectError,
};
use aws_sdk_s3::presigning::PresigningConfigError;
use aws_sdk_s3::primitives::ByteStreamError;

/// An object-storage or signing failure with its original cause.
#[derive(Debug, thiserror::Error)]
pub enum S3Error {
    #[error("S3 put_object failed: {0}")]
    PutObject(#[source] Box<SdkError<PutObjectError>>),
    #[error("S3 get_object failed: {0}")]
    GetObject(#[source] Box<SdkError<GetObjectError>>),
    #[error("Failed to read S3 body: {0}")]
    ReadBody(#[source] ByteStreamError),
    #[error("S3 head_object failed: {0}")]
    HeadObject(#[source] Box<SdkError<HeadObjectError>>),
    #[error("S3 delete_object failed: {0}")]
    DeleteObject(#[source] Box<SdkError<DeleteObjectError>>),
    #[error("S3 list_objects_v2 failed: {0}")]
    ListObjects(#[source] Box<SdkError<ListObjectsV2Error>>),
    #[error("S3 delete_objects failed: {0}")]
    DeleteObjects(#[source] Box<SdkError<DeleteObjectsError>>),
    #[error("S3 delete identifier invalid: {0}")]
    InvalidDeleteIdentifier(#[source] BuildError),
    #[error("S3 delete request invalid: {0}")]
    InvalidDeleteRequest(#[source] BuildError),
    #[cfg(feature = "sts")]
    #[error("STS assume_role failed: {0}")]
    AssumeRole(
        #[source]
        Box<aws_sdk_sts::error::SdkError<aws_sdk_sts::operation::assume_role::AssumeRoleError>>,
    ),
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
    InvalidPresignConfig(#[source] PresigningConfigError),
    #[error("S3 presign put_object failed: {0}")]
    PresignPut(#[source] Box<SdkError<PutObjectError>>),
    #[error("S3 presign get_object failed: {0}")]
    PresignGet(#[source] Box<SdkError<GetObjectError>>),
    #[error("S3 presign delete_object failed: {0}")]
    PresignDelete(#[source] Box<SdkError<DeleteObjectError>>),
    #[error("failed to load AWS credentials for presign TTL: {0}")]
    Credentials(#[source] BoxError),
    #[error("AWS credentials expired before S3 presign")]
    CredentialsExpired,
    #[error("AWS credentials expire too soon for S3 presign")]
    CredentialsExpiring,
    #[error("S3 delete_objects left {} of {attempted} keys: {}", failures.len(), first_failure(failures))]
    BatchDelete {
        attempted: usize,
        failures: Vec<aws_sdk_s3::types::Error>,
    },
}

fn first_failure(failures: &[aws_sdk_s3::types::Error]) -> String {
    failures
        .first()
        .map(|failure| {
            format!(
                "{}: {}",
                failure.key().unwrap_or("?"),
                failure.message().unwrap_or("?")
            )
        })
        .unwrap_or_default()
}
