//! File storage error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum FileStorageError {
    #[error("File not found: {0}")]
    NotFound(String),

    #[error("S3 not configured")]
    S3NotConfigured,

    #[error("Upload failed: {0}")]
    UploadFailed(String),

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
            FileStorageError::UploadFailed(ref msg) => {
                tracing::error!(error = %msg, "File upload failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            FileStorageError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            FileStorageError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            FileStorageError::Db(e) => {
                tracing::error!(error = %e, "Database error in file storage handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
