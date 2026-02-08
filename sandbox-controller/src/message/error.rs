//! Message error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum MessageError {
    #[error("Message not found: {0}")]
    NotFound(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for MessageError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            MessageError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            MessageError::Db(e) => {
                tracing::error!(error = %e, "Database error in message handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            MessageError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
