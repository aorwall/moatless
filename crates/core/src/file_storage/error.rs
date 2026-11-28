//! What an upload operation refuses, and the status each refusal answers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::{error_response, internal_error};

#[derive(Debug, thiserror::Error)]
pub enum FileStorageError {
    #[error("File not found: {0}")]
    NotFound(String),

    #[error("S3 not configured")]
    S3NotConfigured,

    #[error("Upload failed: {0}")]
    UploadFailed(String),

    #[error("Upload failed: {0}")]
    Storage(#[from] moatless_s3::S3Error),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for FileStorageError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            FileStorageError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            FileStorageError::S3NotConfigured => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            FileStorageError::UploadFailed(msg) => {
                tracing::error!(error = %msg, "File upload failed");
                internal_error()
            }
            FileStorageError::Storage(source) => {
                tracing::error!(error = %source, "File storage failed");
                internal_error()
            }
            FileStorageError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            FileStorageError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            FileStorageError::Db(e) => {
                tracing::error!(error = %e, "Database error in file storage handler");
                internal_error()
            }
        };

        error_response(status, message)
    }
}
