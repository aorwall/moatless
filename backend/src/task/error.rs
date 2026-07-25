//! Task error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::access_control::AccessControlError;
use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("Task not found: {0}")]
    NotFound(String),

    #[error("Repository not found: {0}")]
    RepoNotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("{0}")]
    Conflict(String),
}

impl From<crate::sandbox::lifecycle::LifecycleError> for TaskError {
    fn from(value: crate::sandbox::lifecycle::LifecycleError) -> Self {
        use crate::sandbox::lifecycle::LifecycleError as LE;
        match &value {
            LE::TaskNotFound(id) => Self::NotFound(id.clone()),
            // The Closed guard rejected the operation — a client state error.
            LE::NotAllowed { .. } => Self::BadRequest(value.to_string()),
            // Another operation holds the per-Task lock.
            LE::OperationInProgress { .. } => Self::Conflict(value.to_string()),
            _ => Self::Internal(value.to_string()),
        }
    }
}

impl From<AccessControlError> for TaskError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => Self::BadRequest(msg),
            AccessControlError::Forbidden(msg) => Self::Forbidden(msg),
        }
    }
}

impl IntoResponse for TaskError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            TaskError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            TaskError::RepoNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            TaskError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            TaskError::Db(e) => {
                tracing::error!(error = %e, "Database error in task handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            TaskError::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal error in task handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            TaskError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            TaskError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
        };

        error_response(status, message)
    }
}
