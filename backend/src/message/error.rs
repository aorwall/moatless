//! Message error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("Message not found: {0}")]
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

impl IntoResponse for MessageError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            MessageError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            MessageError::CommandNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            MessageError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            MessageError::Db(e) => {
                tracing::error!(error = %e, "Database error in message handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            MessageError::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal error in message handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
        };

        error_response(status, message)
    }
}
