//! Task template error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum TaskTemplateError {
    #[error("Task template not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Access denied: {0}")]
    Forbidden(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for TaskTemplateError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            TaskTemplateError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            TaskTemplateError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            TaskTemplateError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            TaskTemplateError::Db(e) => {
                error!(error = %e, "Task template database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
