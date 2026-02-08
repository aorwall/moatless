//! Agent error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Task not found: {0}")]
    NotFound(String),

    #[error("Command not found: {0}")]
    CommandNotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AgentError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AgentError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AgentError::CommandNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AgentError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AgentError::Db(e) => {
                tracing::error!(error = %e, "Database error in agent handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            AgentError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
