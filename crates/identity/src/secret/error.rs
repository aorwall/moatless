//! Secret-specific errors.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_core::error::{error_response, internal_error};

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("Secret not found: {0}")]
    NotFound(String),

    #[error("Invalid secret request: {0}")]
    BadRequest(String),

    #[error("Access denied: {0}")]
    Forbidden(String),

    #[error("Secret encryption is not configured")]
    EncryptionNotConfigured,

    #[error("Secret encryption failed")]
    EncryptionFailed,

    #[error("Secret decryption failed")]
    DecryptionFailed,

    #[error("External secret references are not resolvable by this backend yet: {0}")]
    ExternalRefUnsupported(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for SecretError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::EncryptionNotConfigured => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Self::EncryptionFailed | Self::DecryptionFailed => {
                tracing::error!(error = %self, "Secret crypto error");
                internal_error()
            }
            Self::ExternalRefUnsupported(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Db(e) => {
                tracing::error!(error = %e, "Secret database error");
                internal_error()
            }
            Self::Internal(msg) => {
                tracing::error!(error = %msg, "Secret internal error");
                internal_error()
            }
        };

        error_response(status, message)
    }
}
