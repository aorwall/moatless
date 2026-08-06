//! Workspace error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use moatless_backend_core::access_control::AccessControlError;
use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("Workspace not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("{0}")]
    Conflict(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Access denied: {0}")]
    Forbidden(String),
}

impl From<AccessControlError> for WorkspaceError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => Self::BadRequest(msg),
            AccessControlError::Forbidden(msg) => Self::Forbidden(msg),
        }
    }
}

impl IntoResponse for WorkspaceError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            WorkspaceError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            WorkspaceError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            WorkspaceError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            WorkspaceError::Db(e) => {
                error!(error = %e, "Workspace database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            WorkspaceError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
        };

        error_response(status, message)
    }
}
