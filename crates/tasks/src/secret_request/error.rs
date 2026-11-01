//! Secret request errors.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};
use moatless_identity::secret::error::SecretError;

#[derive(Debug, thiserror::Error)]
pub enum SecretRequestError {
    /// Also raised for a Task the caller cannot read, so a stranger learns
    /// nothing from the answer.
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Secret request not found: {0}")]
    NotFound(String),

    #[error("Invalid secret request: {0}")]
    BadRequest(String),

    #[error("{0}")]
    Forbidden(String),

    #[error("This secret request was already answered")]
    AlreadyAnswered,

    #[error("Could not store the secret")]
    Store(#[source] SecretError),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for SecretRequestError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::TaskNotFound(_) | Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::AlreadyAnswered => (StatusCode::CONFLICT, self.to_string()),
            Self::Store(_) | Self::Db(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Secret request failed"
                );
                internal_error()
            }
        };
        error_response(status, message)
    }
}
