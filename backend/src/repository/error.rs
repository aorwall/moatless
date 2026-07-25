//! Repository error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::{error, warn};

use crate::access_control::AccessControlError;
use crate::error::error_response;

pub(crate) const REMOTE_URL_UNIQUE_INDEX: &str = "repositories_active_owner_remote_url_uq";

pub(crate) fn is_remote_url_conflict(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database_error| database_error.constraint())
        == Some(REMOTE_URL_UNIQUE_INDEX)
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Repository not found: {0}")]
    NotFound(String),

    #[error("Repository URL already exists: {0}")]
    Conflict(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Provider API error: {0}")]
    ProviderError(String),

    #[error("Access denied: {0}")]
    Forbidden(String),
}

impl RepositoryError {
    pub(crate) fn from_write_error(error: sqlx::Error, remote_url: Option<&str>) -> Self {
        match remote_url {
            Some(remote_url) if is_remote_url_conflict(&error) => {
                Self::Conflict(remote_url.to_string())
            }
            _ => Self::Db(error),
        }
    }
}

impl From<AccessControlError> for RepositoryError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => Self::BadRequest(msg),
            AccessControlError::Forbidden(msg) => Self::Forbidden(msg),
        }
    }
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
                    "Internal server error".to_string(),
                )
            }
            RepositoryError::ProviderError(msg) => {
                warn!(error = %msg, "Repository provider API error");
                (StatusCode::BAD_GATEWAY, self.to_string())
            }
            RepositoryError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
        };

        error_response(status, message)
    }
}
