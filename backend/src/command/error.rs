//! Command error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum CommandError {
    #[error("Command not found: {0}")]
    NotFound(String),

    #[error("Command already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for CommandError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            CommandError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            CommandError::AlreadyExists(_) => (StatusCode::CONFLICT, self.to_string()),
            CommandError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            CommandError::Db(e) => {
                tracing::error!(error = %e, "Database error in command handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
