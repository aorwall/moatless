//! Task event error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};

#[derive(Debug, thiserror::Error)]
pub enum TaskEventError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for TaskEventError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            TaskEventError::TaskNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            TaskEventError::Db(e) => {
                tracing::error!(error = %e, "Database error in task event handler");
                internal_error()
            }
        };
        error_response(status, message)
    }
}
