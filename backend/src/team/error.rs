//! Team module error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use moatless_backend_core::access_control::AccessControlError;
use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum TeamError {
    #[error("Team not found: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<AccessControlError> for TeamError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => Self::BadRequest(msg),
            AccessControlError::Forbidden(msg) => Self::Forbidden(msg),
        }
    }
}

impl IntoResponse for TeamError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            Self::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            Self::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            Self::Db(e) => {
                tracing::error!(error = %e, "Database error in team handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::Internal(m) => {
                tracing::error!(error = %m, "Internal error in team handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };
        error_response(status, message)
    }
}
