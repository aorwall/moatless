//! Repository error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Repository not found: {0}")]
    NotFound(String),

    #[error("Repository name already exists: {0}")]
    Conflict(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Provider API error: {0}")]
    ProviderError(String),
}

impl IntoResponse for RepositoryError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            RepositoryError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            RepositoryError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            RepositoryError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            RepositoryError::Db(e) => {
                error!(error = %e, "Repository database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal database error".to_string(),
                )
            }
            RepositoryError::ProviderError(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
