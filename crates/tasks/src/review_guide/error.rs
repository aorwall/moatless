//! Guided-review error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};

#[derive(Debug, thiserror::Error)]
pub enum ReviewGuideError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// The Task has no running guide run: none was started for it, a newer run
    /// replaced it, or it was marked failed.
    #[error("{0}")]
    NoRun(String),

    /// The submitted guide cannot be stored; the message tells the agent what
    /// to fix before it submits again.
    #[error("{0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// The guide could not be encoded for storage.
    #[error("Guide could not be encoded")]
    Encode(#[from] serde_json::Error),
}

impl IntoResponse for ReviewGuideError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::TaskNotFound(_) | Self::NoRun(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Db(_) | Self::Encode(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Review guide handler failed"
                );
                internal_error()
            }
        };
        error_response(status, message)
    }
}
