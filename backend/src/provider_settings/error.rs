//! Provider settings error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum ProviderSettingsError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Secret storage error: {0}")]
    Secret(#[from] crate::secret::error::SecretError),
}

impl IntoResponse for ProviderSettingsError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::Db(e) => {
                error!(error = %e, "Provider settings database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::ProviderError(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            Self::Secret(e) => {
                error!(error = %e, "Provider settings secret storage error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to store credential securely".to_string(),
                )
            }
        };
        error_response(status, message)
    }
}
