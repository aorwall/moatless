//! Workspace error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use moatless_core::error::{error_response, internal_error};
use moatless_types::access_control::AccessControlError;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("Workspace not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("{0}")]
    Conflict(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Access denied: {0}")]
    Forbidden(String),

    /// No `SECRETS_ENCRYPTION_KEY` is configured, so workspace credential
    /// values can be neither stored nor read. A deployment gap rather than a
    /// bad request, hence 503.
    #[error("Secret encryption is not configured")]
    EncryptionNotConfigured,

    /// Envelope encryption or decryption of a workspace credential failed. The
    /// cause is logged where it happens; the client sees nothing about it.
    #[error("Credential encryption error")]
    Crypto,
}

impl From<AccessControlError> for WorkspaceError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => Self::BadRequest(msg),
            AccessControlError::Forbidden(msg) => Self::Forbidden(msg),
        }
    }
}

impl IntoResponse for WorkspaceError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            WorkspaceError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            WorkspaceError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            WorkspaceError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            WorkspaceError::Db(e) => {
                error!(error = %e, "Workspace database error");
                internal_error()
            }
            WorkspaceError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            WorkspaceError::EncryptionNotConfigured => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            WorkspaceError::Crypto => internal_error(),
        };

        error_response(status, message)
    }
}
