//! Task error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

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
}

impl IntoResponse for TaskError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            TaskError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            TaskError::RepoNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            TaskError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            TaskError::Db(e) => {
                tracing::error!(error = %e, "Database error in task handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            TaskError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
