//! Artifact error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    /// Also raised for an Artifact on a Task the caller cannot read, so the
    /// response does not tell a stranger it exists.
    #[error("Artifact not found: {0}")]
    NotFound(String),

    /// The caller can read the Task but not change it.
    #[error("Not allowed to publish on task {0}")]
    Forbidden(String),

    /// A page token that is missing, expired, or not signed here.
    #[error("Invalid or expired artifact page token")]
    Unauthorized,

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("Artifact storage is not configured")]
    StorageNotConfigured,

    #[error("Object storage error")]
    Storage(#[from] moatless_s3::S3Error),

    #[error("Database error")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for ArtifactError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::TaskNotFound(_) | Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            Self::StorageNotConfigured => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Self::Storage(_) | Self::Db(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Storage error in artifact handler"
                );
                internal_error()
            }
        };

        error_response(status, message)
    }
}
