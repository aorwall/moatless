//! Command error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("Command not found: {0}")]
    NotFound(String),

    #[error("Command already exists: {0}")]
    AlreadyExists(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for CommandError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            CommandError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            CommandError::AlreadyExists(_) => (StatusCode::CONFLICT, self.to_string()),
            CommandError::Db(e) => {
                tracing::error!(error = %e, "Database error in command handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
