//! Memory error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    /// The caller can read the Task but not write memory for it.
    #[error("Not allowed: {0}")]
    Forbidden(String),

    #[error("Memory entry not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    /// A stale `ifVersion`, or a claim another open Task holds.
    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Database error")]
    Db(#[from] sqlx::Error),

    /// Reading the Workspace or repository a scope belongs to failed.
    #[error("Store error")]
    Store(#[from] moatless_types::store::StoreError),
}

impl IntoResponse for MemoryError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::TaskNotFound(_) | Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            Self::Db(_) | Self::Store(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Database error in memory handler"
                );
                internal_error()
            }
        };

        error_response(status, message)
    }
}
