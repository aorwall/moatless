//! Plugin repo error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum PluginRepoError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl From<crate::access_control::AccessControlError> for PluginRepoError {
    fn from(e: crate::access_control::AccessControlError) -> Self {
        use crate::access_control::AccessControlError as E;
        match e {
            E::BadRequest(m) => PluginRepoError::BadRequest(m),
            E::Forbidden(m) => PluginRepoError::Forbidden(m),
        }
    }
}

impl IntoResponse for PluginRepoError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            PluginRepoError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            PluginRepoError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            PluginRepoError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            PluginRepoError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            PluginRepoError::Db(e) => {
                tracing::error!(error = %e, "Database error in plugin_repo handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
