//! Tag error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for TagError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            TagError::Db(e) => {
                tracing::error!(error = %e, "Database error in tag handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            TagError::Internal(msg) => {
                tracing::error!(error = %msg, "Internal error in tag handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
