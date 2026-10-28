//! Record collection error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};

use super::keys::InvalidRecord;
use crate::scope_reach::ScopeError;

#[derive(Debug, thiserror::Error)]
pub enum CollectionError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    /// The caller can read the Task but not write the scope's records.
    #[error("Not allowed: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("Database error")]
    Db(#[from] sqlx::Error),

    /// Reading the Workspace or repository a scope belongs to failed.
    #[error("Store error")]
    Store(#[from] moatless_types::store::StoreError),
}

impl From<ScopeError> for CollectionError {
    fn from(error: ScopeError) -> Self {
        match error {
            ScopeError::Forbidden(message) => Self::Forbidden(message),
            ScopeError::Missing(message) => Self::BadRequest(message),
            ScopeError::Store(error) => Self::Store(error),
        }
    }
}

impl From<InvalidRecord> for CollectionError {
    fn from(error: InvalidRecord) -> Self {
        match error {
            InvalidRecord::Invalid(message) => Self::BadRequest(message),
            InvalidRecord::TooLarge(message) => Self::PayloadTooLarge(message),
        }
    }
}

impl IntoResponse for CollectionError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::TaskNotFound(_) | Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            Self::Db(_) | Self::Store(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Database error in collection handler"
                );
                internal_error()
            }
        };

        error_response(status, message)
    }
}
