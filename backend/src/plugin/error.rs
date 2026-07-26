//! Plugin error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
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

impl From<crate::access_control::AccessControlError> for PluginError {
    fn from(e: crate::access_control::AccessControlError) -> Self {
        use crate::access_control::AccessControlError as E;
        match e {
            E::BadRequest(m) => PluginError::BadRequest(m),
            E::Forbidden(m) => PluginError::Forbidden(m),
        }
    }
}

impl IntoResponse for PluginError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            PluginError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            PluginError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            PluginError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            PluginError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            PluginError::Db(e) => {
                tracing::error!(error = %e, "Database error in plugin handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
