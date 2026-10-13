//! Repository error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::{error, warn};

use moatless_core::error::{error_response, internal_error};
use moatless_types::access_control::AccessControlError;

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

    /// A published store seam failed. Distinct from [`Self::Db`] only in where
    /// the failure was raised; both are an internal error to a caller.
    #[error("{0}")]
    Store(#[from] moatless_types::store::StoreError),

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
                internal_error()
            }
            RepositoryError::Store(e) => {
                error!(error = %moatless_core::error::causal_chain(e), "Repository store error");
                internal_error()
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
