//! Feedback error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum FeedbackError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),
}

impl IntoResponse for FeedbackError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::TaskNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Db(e) => {
                tracing::error!(error = %e, "Database error in feedback handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::Internal(msg) => {
                tracing::error!(error = %msg, "Internal error in feedback handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
        };

        error_response(status, message)
    }
}
