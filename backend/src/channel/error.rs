//! Channel error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("Channel config not found: {0}")]
    ConfigNotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Provider not configured: {0}")]
    ProviderNotConfigured(String),

    #[error("Signature verification failed")]
    SignatureInvalid,

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Provider API error: {0}")]
    ProviderApi(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ChannelError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ChannelError::ConfigNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ChannelError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ChannelError::ProviderNotConfigured(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ChannelError::SignatureInvalid => (StatusCode::UNAUTHORIZED, self.to_string()),
            ChannelError::Db(e) => {
                tracing::error!(error = %e, "Database error in channel handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            ChannelError::AccessDenied(_) => (StatusCode::FORBIDDEN, self.to_string()),
            ChannelError::ProviderApi(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            ChannelError::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal error in channel handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
