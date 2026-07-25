//! Adapter error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use moatless_api_schemas::bindings::{MultipleTargetsErrorBody, ReplyTargetDescriptor};

use crate::access_control::AccessControlError;
use crate::error::error_response;

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("Adapter config not found: {0}")]
    ConfigNotFound(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Adapter not configured: {0}")]
    AdapterNotConfigured(String),

    #[error("Signature verification failed")]
    SignatureInvalid,

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Adapter API error: {0}")]
    AdapterApi(String),

    #[error("Adapter API error: {0}")]
    Unauthorized(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    /// Returned by the no-flag send endpoint when a task has more than one
    /// reply target — the caller must pick one with `--thread`,
    /// `--binding`, or `--pr`.
    #[error("Task has multiple reply targets; pick one explicitly")]
    MultipleTargets { targets: Vec<ReplyTargetDescriptor> },

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<AccessControlError> for AdapterError {
    fn from(value: AccessControlError) -> Self {
        match value {
            AccessControlError::BadRequest(msg) => Self::BadRequest(msg),
            AccessControlError::Forbidden(msg) => Self::AccessDenied(msg),
        }
    }
}

impl IntoResponse for AdapterError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AdapterError::ConfigNotFound(_) | AdapterError::NotFound(_) => {
                (StatusCode::NOT_FOUND, self.to_string())
            }
            AdapterError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AdapterError::AdapterNotConfigured(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AdapterError::SignatureInvalid => (StatusCode::UNAUTHORIZED, self.to_string()),
            AdapterError::Db(e) => {
                tracing::error!(error = %e, "Database error in adapter handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AdapterError::AccessDenied(_) => (StatusCode::FORBIDDEN, self.to_string()),
            AdapterError::AdapterApi(_) | AdapterError::Unauthorized(_) => {
                (StatusCode::BAD_GATEWAY, self.to_string())
            }
            AdapterError::MultipleTargets { targets } => {
                let body = MultipleTargetsErrorBody {
                    error: self.to_string(),
                    targets: targets.clone(),
                    hint: "Use --binding <binding_id> to pick one.".to_string(),
                };
                return (StatusCode::BAD_REQUEST, Json(body)).into_response();
            }
            AdapterError::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal error in adapter handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        error_response(status, message)
    }
}
