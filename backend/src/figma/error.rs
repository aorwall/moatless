//! Figma error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum FigmaError {
    #[error("Import not found: {0}")]
    NotFound(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Figma API error: {0}")]
    ApiError(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Storage error: {0}")]
    StorageError(String),
}

impl IntoResponse for FigmaError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            FigmaError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            FigmaError::ParseError(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            FigmaError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            FigmaError::ApiError(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            FigmaError::Db(e) => {
                error!(error = %e, "Figma database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            FigmaError::StorageError(ref msg) => {
                error!(error = %msg, "Figma storage error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
        };

        error_response(status, message)
    }
}
